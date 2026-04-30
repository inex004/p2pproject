mod crypto;
mod network;
mod auction;

use std::fs;
use curve25519_dalek::scalar::Scalar;
use rand::thread_rng;
use std::error::Error;
use tokio::{io, io::AsyncBufReadExt, select, time}; 
use futures::StreamExt; 
use libp2p::swarm::SwarmEvent;
use std::env;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH, Duration}; 

const HEARTBEAT_TIMEOUT: u64 = 15; 

const AUTHORIZED_METERS: [&str; 4] = [
    "12D3KooWP12edPP1guWsgxgmr74Lt1aE7JwFksyCiew9Srr8RjwB", 
    "12D3KooWFuoRX7BQ9PJHUxzvJjzuJFx11TYPWX6A3pRWqUHxZZeg", 
    "12D3KooWCaSszh4dejZ2zWaRUfeXadBmsrvXmRZDokghX2pCSUf9", 
    "12D3KooWMvGUTxq75wzFwMy7YjKaKigV4w2UxqcjTbNvUDaxXWHs", 
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("!!! THE PROGRAM HAS SUCCESSFULLY COMPILED AND STARTED !!!");

    let args: Vec<String> = env::args().collect();
    let listen_port: u16 = args[1].parse().expect("Invalid port");
    
    let key_file = format!("meter_{}.key", listen_port);
    let id_keys = if let Ok(bytes) = fs::read(&key_file) {
        libp2p::identity::Keypair::from_protobuf_encoding(&bytes).unwrap()
    } else {
        let new_key = libp2p::identity::Keypair::generate_ed25519();
        fs::write(&key_file, new_key.to_protobuf_encoding().unwrap()).unwrap();
        new_key
    };

    let local_peer_id = id_keys.public().to_peer_id();
    let mut swarm = network::setup_swarm(id_keys, local_peer_id)?;
    let listen_addr: libp2p::Multiaddr = format!("/ip4/127.0.0.1/tcp/{}", listen_port).parse()?;
    
    swarm.listen_on(listen_addr.clone())?;
    swarm.behaviour_mut().kademlia.set_mode(Some(libp2p::kad::Mode::Server));

    println!("=========================================================");
    println!("      ⚡ DECENTRALIZED P2P ENERGY MARKETPLACE ⚡      ");
    println!("=========================================================");
    println!("My Permanent Peer ID: {}", local_peer_id);
    println!("📡 Listening on: {}", listen_addr);
    println!("---------------------------------------------------------");
    println!("COMMANDS:");
    println!("  WALLET                   - View your Credits and Energy balances");
    println!("  SELL <energy> <reserve>  - Create auction (e.g., SELL 100 50)");
    println!("  LOBBY                    - View all active auctions on the network");
    println!("  JOIN <id>                - Select an auction to participate in");
    println!("  BID <amount>             - Submit a cryptographic bid");
    println!("  AUDIT <id>               - Act as a Network Validator");
    println!("  UNPLUG <id>              - (Seller Only) Physically disconnect your meter");
    println!("---------------------------------------------------------");

    if args.len() > 2 {
        let bootstrap_addr: libp2p::Multiaddr = args[2].parse().unwrap();
        println!("🔗 Bootstrapping... Dialing known neighbor: {}", bootstrap_addr);
        swarm.dial(bootstrap_addr)?;
    }

    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let mut state = auction::MarketplaceState::new();
    let topic = libp2p::gossipsub::IdentTopic::new("energy-auction");

    let mut background_timer = time::interval(Duration::from_secs(1));
    let mut current_audited_auction: Option<String> = None;
    let mut last_heartbeat_sent = 0;

    loop {
        select! {
            _ = background_timer.tick() => {
                let current_unix_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                let mut auctions_to_resolve = Vec::new();

                for (auction_id, current_auction) in state.active_auctions.iter_mut() {
                    
                    if current_auction.is_delivering && !current_auction.slashed {
                        if current_auction.seller_id == local_peer_id.to_string() {
                            if !state.unplugged_meters.contains(auction_id) {
                                if current_unix_secs.saturating_sub(last_heartbeat_sent) >= 5 {
                                    
                                    current_auction.energy_delivered += 20;

                                    if current_auction.energy_delivered >= current_auction.energy_amount {
                                        println!("✅ [SMART METER]: Delivery of {} kWh Complete! Shutting down flow.", current_auction.energy_amount);
                                        current_auction.is_delivering = false;
                                        
                                        let msg = network::NetworkMessage::DeliveryComplete {
                                            auction_id: auction_id.clone(),
                                            seller_id: local_peer_id.to_string(),
                                        };
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), serde_json::to_string(&msg).unwrap().as_bytes());
                                    } else {
                                        println!("⚡ [SMART METER]: Routing energy... {}/{} kWh delivered.", current_auction.energy_delivered, current_auction.energy_amount);
                                        let msg = network::NetworkMessage::Heartbeat {
                                            auction_id: auction_id.clone(),
                                            seller_id: local_peer_id.to_string(),
                                        };
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), serde_json::to_string(&msg).unwrap().as_bytes());
                                        last_heartbeat_sent = current_unix_secs;
                                    }
                                }
                            }
                        } else {
                            if current_unix_secs.saturating_sub(current_auction.last_heartbeat) > HEARTBEAT_TIMEOUT {
                                println!("\n🚨🚨🚨 CRITICAL ORACLE FAILURE DETECTED 🚨🚨🚨");
                                println!("Meter {} stopped sending energy flow heartbeats!", current_auction.seller_id);
                                println!("⚖️ EXECUTING SMART CONTRACT SLASHING CONDITION:");
                                
                                if current_auction.seller_id == local_peer_id.to_string() {
                                    state.my_locked_credits -= auction::STAKE_AMOUNT; // Burned!
                                    println!("   [-] Burning your {} Escrowed Credits...", auction::STAKE_AMOUNT);
                                }
                                if Some(local_peer_id.to_string()) == current_auction.winner_id {
                                    state.my_credits += 100; // Compensation
                                    println!("   [+] You received 100 credits as slashing compensation.");
                                }
                                
                                current_auction.slashed = true;
                                current_auction.is_delivering = false;
                                
                                if let Ok(peer_id_obj) = current_auction.seller_id.parse() {
                                    let _ = swarm.disconnect_peer_id(peer_id_obj);
                                }
                            }
                        }
                    }

                    if current_unix_secs > current_auction.commit_deadline && !current_auction.resolved {
                        let mut unrevealed: Vec<&String> = current_auction.received_commitments.keys()
                            .filter(|id| !current_auction.verified_bids.contains_key(*id)).collect();
                        unrevealed.sort();

                        if unrevealed.is_empty() && !current_auction.received_commitments.is_empty() {
                            auctions_to_resolve.push(auction_id.clone());
                        } 
                        else if let Some(expected_peer) = unrevealed.first() {
                            let expected_peer_cloned = expected_peer.to_string();
                            if expected_peer_cloned == local_peer_id.to_string() && !current_auction.verified_bids.contains_key(&local_peer_id.to_string()) {
                                if Some(auction_id.clone()) == state.current_joined_auction {
                                    if let (Some(bid), Some(blind)) = (state.my_secret_bid, state.my_secret_blind) {
                                        let msg = network::NetworkMessage::Reveal {
                                            auction_id: auction_id.clone(), bidder_id: local_peer_id.to_string(),
                                            bid, blind_hex: hex::encode(blind.as_bytes()),
                                        };
                                        current_auction.verified_bids.insert(local_peer_id.to_string(), bid);
                                        let mut blind_bytes = [0u8; 32]; blind_bytes.copy_from_slice(blind.as_bytes());
                                        current_auction.verified_blinds.insert(local_peer_id.to_string(), blind_bytes);
                                        let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), serde_json::to_string(&msg).unwrap().as_bytes());
                                    }
                                }
                            }
                        }
                    }
                }

                for auction_id in auctions_to_resolve {
                    if let Some(auction_to_close) = state.active_auctions.get_mut(&auction_id) {
                        auction_to_close.resolve();
                        
                        let my_id = local_peer_id.to_string();
                        
                        if auction_to_close.failed {
                            if auction_to_close.seller_id == my_id {
                                state.my_locked_credits -= auction::STAKE_AMOUNT;
                                state.my_credits += auction::STAKE_AMOUNT;
                                state.my_locked_energy -= auction_to_close.energy_amount;
                                state.my_energy += auction_to_close.energy_amount;
                                println!("💼 Wallet: Escrow & Energy refunded (Market Failed).");
                            }
                            if let Some(my_bid) = auction_to_close.verified_bids.get(&my_id) {
                                state.my_locked_credits -= my_bid;
                                state.my_credits += my_bid;
                                println!("💼 Wallet: Bid of {} refunded.", my_bid);
                            }
                        } else {
                            let winner = auction_to_close.winner_id.as_ref().unwrap();
                            let price = auction_to_close.clearing_price;
                            
                            if auction_to_close.seller_id == my_id {
                                state.my_locked_credits -= auction::STAKE_AMOUNT;
                                state.my_credits += auction::STAKE_AMOUNT + price;
                                state.my_locked_energy -= auction_to_close.energy_amount; 
                                println!("💼 Wallet: Sold! Received {} credits + Escrow returned.", price);
                            }
                            
                            if winner == &my_id {
                                let my_bid = auction_to_close.verified_bids.get(&my_id).unwrap();
                                state.my_locked_credits -= my_bid;
                                state.my_credits += my_bid - price; 
                                state.my_energy += auction_to_close.energy_amount; 
                                println!("💼 Wallet/Battery: Won! Paid {} credits. Received {} kWh.", price, auction_to_close.energy_amount);
                            } else if let Some(my_bid) = auction_to_close.verified_bids.get(&my_id) {
                                state.my_locked_credits -= my_bid;
                                state.my_credits += my_bid;
                                println!("💼 Wallet: Lost auction. Bid of {} refunded.", my_bid);
                            }
                        }
                    }
                }
            }

            Ok(Some(line)) = stdin.next_line() => {
                let line_str = line.trim(); 
                let parts: Vec<&str> = line_str.split_whitespace().collect();
                
                if line_str == "WALLET" {
                    println!("\n💰 YOUR VIRTUAL LEDGER 💰");
                    println!("   Credits: {} (Locked in Escrow: {})", state.my_credits, state.my_locked_credits);
                    println!("   Battery: {} kWh (Locked for Sale: {} kWh)", state.my_energy, state.my_locked_energy);
                    println!("");
                }
                // 🔥 NEW: Parses both energy amount AND reserve price
                else if line_str.starts_with("SELL ") && parts.len() == 3 {
                    if let (Ok(energy_amount), Ok(reserve_price)) = (parts[1].parse::<u64>(), parts[2].parse::<u64>()) {
                        if state.my_credits < auction::STAKE_AMOUNT {
                            println!("❌ Error: You need {} credits for the Escrow Stake to sell.", auction::STAKE_AMOUNT);
                        } else if state.my_energy < energy_amount {
                            println!("❌ Error: You only have {} kWh in your battery.", state.my_energy);
                        } else {
                            state.my_credits -= auction::STAKE_AMOUNT;
                            state.my_locked_credits += auction::STAKE_AMOUNT;
                            state.my_energy -= energy_amount;
                            state.my_locked_energy += energy_amount;
                            
                            let current_unix_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                            let new_auction_id = format!("AUC_{}", current_unix_secs); 
                            
                            println!("🔒 SMART CONTRACT: Locking {} credits and {} kWh into Escrow...", auction::STAKE_AMOUNT, energy_amount);
                            println!("📢 Creating Market ID: {} (Reserve: {} credits)", new_auction_id, reserve_price);
                            
                            let new_auction = auction::Auction::new(new_auction_id.clone(), local_peer_id.to_string(), energy_amount, reserve_price);
                            state.active_auctions.insert(new_auction_id.clone(), new_auction);
                            
                            let msg = network::NetworkMessage::AnnounceAuction {
                                auction_id: new_auction_id, seller_id: local_peer_id.to_string(), energy_amount, reserve_price,
                            };
                            let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), serde_json::to_string(&msg).unwrap().as_bytes());
                        }
                    } else {
                        println!("❌ Error: Invalid format. Use: SELL <energy> <reserve>");
                    }
                } 
                else if line_str == "LOBBY" {
                    println!("\n🏛️  GLOBAL MARKETPLACE LOBBY 🏛️");
                    for (id, a) in &state.active_auctions {
                        let status = if a.is_delivering { "DELIVERING" } else if a.resolved { "CLOSED" } else { "OPEN" };
                        // 🔥 NEW: Shows the reserve price in the Lobby
                        println!("   ID: {} | Seller: {}... | {} kWh | Reserve: {} | Status: {}", id, &a.seller_id[0..8], a.energy_amount, a.reserve_price, status);
                    }
                } 
                else if line_str.starts_with("JOIN ") && parts.len() == 2 {
                    state.current_joined_auction = Some(parts[1].to_string());
                    println!("✅ Joined auction {}.", parts[1]);
                }
                else if line_str.starts_with("AUDIT ") && parts.len() == 2 {
                    current_audited_auction = Some(parts[1].to_string());
                    println!("🔍 SECURE AUDIT MODE ACTIVATED for {}.", parts[1]);
                }
                else if line_str.starts_with("UNPLUG ") && parts.len() == 2 {
                    let target_id = parts[1].to_string();
                    println!("💥 WARNING: You have physically unplugged your smart meter for {}!", target_id);
                    state.unplugged_meters.insert(target_id);
                }
                else if line_str.starts_with("BID ") && parts.len() == 2 {
                    if let (Some(joined_id), Ok(bid_amount)) = (&state.current_joined_auction, parts[1].parse::<u64>()) {
                        
                        if state.my_credits < bid_amount {
                            println!("❌ Error: Insufficient funds. You only have {} credits.", state.my_credits);
                        } else {
                            state.my_credits -= bid_amount;
                            state.my_locked_credits += bid_amount;
                            
                            let mut rng = thread_rng();
                            let r = Scalar::random(&mut rng);
                            state.my_secret_bid = Some(bid_amount);
                            state.my_secret_blind = Some(r);
                            
                            let my_commitment = crypto::commit(bid_amount, r);
                            let commitment_hex = hex::encode(my_commitment.compress().as_bytes());
                            let my_binding_hash = crypto::generate_binding_hash(&commitment_hex, &local_peer_id.to_string());
                            
                            if let Some(joined_auction) = state.active_auctions.get_mut(joined_id) {
                                joined_auction.received_commitments.insert(local_peer_id.to_string(), my_binding_hash.clone());
                            }
                            
                            let msg = network::NetworkMessage::Commit {
                                auction_id: joined_id.clone(), bidder_id: local_peer_id.to_string(), binding_hash: my_binding_hash, 
                            };
                            let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), serde_json::to_string(&msg).unwrap().as_bytes());
                            println!("🔒 SMART CONTRACT: Locked {} credits. Sent Binding Hash to Market {}!", bid_amount, joined_id);
                        }
                    }
                }
            }

            event = swarm.select_next_some() => match event {
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                },
                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Gossipsub(libp2p::gossipsub::Event::Message { message, .. })) => {
                    if let Ok(parsed_msg) = serde_json::from_str::<network::NetworkMessage>(&String::from_utf8_lossy(&message.data)) {
                        
                        if let network::NetworkMessage::Heartbeat { auction_id, seller_id } = &parsed_msg {
                            if let Some(target_auction) = state.active_auctions.get_mut(auction_id) {
                                if target_auction.is_delivering && target_auction.seller_id == *seller_id {
                                    target_auction.last_heartbeat = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                                    if Some(auction_id) == state.current_joined_auction.as_ref() || Some(auction_id) == current_audited_auction.as_ref() {
                                        println!("   💙 [ORACLE LOG] Verified incoming energy flow from {}...", &seller_id[0..8]);
                                    }
                                }
                            }
                            continue;
                        }

                        if let network::NetworkMessage::DeliveryComplete { auction_id, seller_id } = &parsed_msg {
                            if let Some(target_auction) = state.active_auctions.get_mut(auction_id) {
                                if target_auction.is_delivering && target_auction.seller_id == *seller_id {
                                    target_auction.is_delivering = false;
                                    if Some(auction_id) == state.current_joined_auction.as_ref() || Some(auction_id) == current_audited_auction.as_ref() {
                                        println!("🎉 [ORACLE LOG] Seller has completed the physical energy transfer. Smart Contract CLOSED.");
                                    }
                                }
                            }
                            continue;
                        }

                        let incoming_peer_id = match &parsed_msg {
                            network::NetworkMessage::AnnounceAuction { seller_id, .. } => seller_id,
                            network::NetworkMessage::Commit { bidder_id, .. } => bidder_id,
                            network::NetworkMessage::Reveal { bidder_id, .. } => bidder_id,
                            _ => continue,
                        };

                        if !AUTHORIZED_METERS.contains(&incoming_peer_id.as_str()) { continue; }

                        match parsed_msg {
                            // 🔥 NEW: Parses the reserve price from the broadcast
                            network::NetworkMessage::AnnounceAuction { auction_id, seller_id, energy_amount, reserve_price } => {
                                if !state.active_auctions.contains_key(&auction_id) {
                                    println!("📢 NEW MARKET: {}... is selling {} kWh! (Reserve: {}, ID: {})", &seller_id[0..8], energy_amount, reserve_price, auction_id);
                                    let new_auction = auction::Auction::new(auction_id.clone(), seller_id, energy_amount, reserve_price);
                                    state.active_auctions.insert(auction_id, new_auction);
                                }
                            },
                            network::NetworkMessage::Commit { auction_id, bidder_id, binding_hash } => {
                                if let Some(target_auction) = state.active_auctions.get_mut(&auction_id) {
                                    if !target_auction.received_commitments.contains_key(&bidder_id) {
                                        target_auction.received_commitments.insert(bidder_id, binding_hash);
                                    }
                                }
                            },
                            network::NetworkMessage::Reveal { auction_id, bidder_id, bid, blind_hex } => {
                                if let Some(target_auction) = state.active_auctions.get_mut(&auction_id) {
                                    if let Some(stored_binding_hash) = target_auction.received_commitments.get(&bidder_id) {
                                        if crypto::verify_binding_hash(stored_binding_hash, bid, &blind_hex, &bidder_id) {
                                            target_auction.verified_bids.insert(bidder_id.clone(), bid);
                                            if let Ok(blind_bytes_vec) = hex::decode(&blind_hex) {
                                                let mut blind_bytes = [0u8; 32];
                                                blind_bytes.copy_from_slice(&blind_bytes_vec);
                                                target_auction.verified_blinds.insert(bidder_id.clone(), blind_bytes);
                                            }
                                        }
                                    }
                                }
                            },
                            _ => {}
                        }
                    }
                },
                _ => {}
            }
        }
    }
}