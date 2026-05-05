mod crypto;
mod network;
mod auction;

use std::fs;
use std::collections::HashSet;
use curve25519_dalek::scalar::Scalar;
use rand::thread_rng;
use std::error::Error;
use tokio::{io, io::AsyncBufReadExt, select, time}; 
use futures::StreamExt; 
use libp2p::swarm::SwarmEvent;
use libp2p::multiaddr::Protocol;
use libp2p::kad::RecordKey;
use std::env;
use std::time::{SystemTime, UNIX_EPOCH, Duration}; 

const HEARTBEAT_TIMEOUT: u64 = 600;
const RELAY_PEER_ID: &str = "12D3KooWKxwfyfV85s6H4PP91L7TgdVrbMS9VDRasyBAio5hKy8S"; 

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
    
    let listen_addr: libp2p::Multiaddr = format!("/ip4/0.0.0.0/udp/{}/quic-v1", listen_port).parse()?;
    swarm.listen_on(listen_addr.clone())?;

    println!("=========================================================");
    println!("      ⚡ DECENTRALIZED P2P ENERGY MARKETPLACE ⚡      ");
    println!("=========================================================");
    println!("My Permanent Peer ID: {}", local_peer_id);
    println!("---------------------------------------------------------");

    let mut global_relay_addr: Option<libp2p::Multiaddr> = None;

    if args.len() > 2 {
        let bootstrap_addr: libp2p::Multiaddr = args[2].parse().unwrap();
        println!("🔗 Dialing Cloud Relay... (Waiting for secure TCP connection)");
        
        swarm.dial(bootstrap_addr.clone())?;
        global_relay_addr = Some(bootstrap_addr);
    }

    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let mut state = auction::MarketplaceState::new();
    let topic = libp2p::gossipsub::IdentTopic::new("energy-auction");
    let mut background_timer = time::interval(Duration::from_secs(10));
    let mut last_heartbeat_sent = 0;
    let mut has_revealed = false; 
    let mut executed_auctions = std::collections::HashSet::new(); 
    
    let mut known_peers = HashSet::new();
    let room_key = RecordKey::new(&"energy-auction");

    loop {
        select! {
            _ = background_timer.tick() => {
                let current_unix_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                let mut local_resolution_queue = Vec::new();

                if global_relay_addr.is_some() {
                    let _ = swarm.behaviour_mut().kad.get_providers(room_key.clone());
                }

                for (auction_id, auction) in &state.active_auctions {
                    if !auction.verdict_received {
                        let is_seller = auction.seller_id == local_peer_id.to_string();
                        let is_joined = state.current_joined_auction.as_ref() == Some(auction_id);
                        let is_validator = auction.validator_id.as_ref() == Some(&local_peer_id.to_string());

                        if is_seller || is_joined || is_validator {
                            let role = if is_seller { "SELLER" } else if is_validator { "REFEREE" } else { "BUYER" };

                            if current_unix_secs <= auction.commit_deadline {
                                let remaining = auction.commit_deadline.saturating_sub(current_unix_secs);
                                if remaining > 0 && remaining % 60 == 0 { 
                                    println!("⏱️ [{} VIEW | {}]: {} minutes remaining in COMMIT Phase.", role, auction_id, remaining / 60);
                                } else if remaining == 10 {
                                    println!("⚠️ [{} VIEW | {}]: 10 SECONDS LEFT IN COMMIT PHASE!", role, auction_id);
                                }
                            } else if current_unix_secs <= auction.reveal_deadline {
                                let remaining = auction.reveal_deadline.saturating_sub(current_unix_secs);
                                if remaining == 45 {
                                    println!("⏱️ [{} VIEW | {}]: Bidding Closed! Entering REVEAL Phase (45s left).", role, auction_id);
                                } else if remaining == 10 {
                                    println!("⚠️ [{} VIEW | {}]: 10 SECONDS TO REVEAL! Network Referee preparing verdict...", role, auction_id);
                                }
                            }
                        }
                    }
                }

                for (auction_id, current_auction) in state.active_auctions.iter_mut() {
                    if current_auction.is_delivering && !current_auction.slashed {
                        if current_auction.seller_id == local_peer_id.to_string() {
                            if !state.unplugged_meters.contains(auction_id) && current_unix_secs.saturating_sub(last_heartbeat_sent) >= 5 {
                                current_auction.energy_delivered += 50; 
                                if current_auction.energy_delivered >= current_auction.energy_amount {
                                    println!("✅ [SMART METER]: Delivery Complete!");
                                    current_auction.is_delivering = false;
                                    let msg = network::NetworkMessage::DeliveryComplete { auction_id: auction_id.clone(), seller_id: local_peer_id.to_string() };
                                    let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), serde_json::to_string(&msg).unwrap().as_bytes());
                                } else {
                                    println!("⚡ [SMART METER]: Routing energy... {}/{} kWh", current_auction.energy_delivered, current_auction.energy_amount);
                                    let msg = network::NetworkMessage::Heartbeat { auction_id: auction_id.clone(), seller_id: local_peer_id.to_string() };
                                    let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), serde_json::to_string(&msg).unwrap().as_bytes());
                                    last_heartbeat_sent = current_unix_secs;
                                }
                            }
                        } else if current_unix_secs.saturating_sub(current_auction.last_heartbeat) > HEARTBEAT_TIMEOUT {
                            println!("\n🚨🚨🚨 CRITICAL ORACLE FAILURE DETECTED 🚨🚨🚨");
                            let penalty = current_auction.reserve_price * 2;
                            if current_auction.seller_id == local_peer_id.to_string() {
                                state.my_locked_credits -= penalty;
                            }
                            if Some(local_peer_id.to_string()) == current_auction.winner_id {
                                state.my_credits += 100;
                            }
                            current_auction.slashed = true;
                            current_auction.is_delivering = false;
                        }
                    }

                    if current_unix_secs > current_auction.commit_deadline && current_unix_secs <= current_auction.reveal_deadline && !current_auction.resolved {
                        if Some(auction_id.clone()) == state.current_joined_auction && !has_revealed {
                            if let (Some(bid), Some(blind)) = (state.my_secret_bid, state.my_secret_blind) {
                                println!("🔓 REVEAL PHASE: Broadcasting Bid and Random Value...");
                                let msg = network::NetworkMessage::Reveal {
                                    auction_id: auction_id.clone(), bidder_id: local_peer_id.to_string(),
                                    bid, blind_hex: hex::encode(blind.as_bytes()),
                                };
                                let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), serde_json::to_string(&msg).unwrap().as_bytes());
                                current_auction.verified_bids.insert(local_peer_id.to_string(), bid);
                                let mut blind_bytes = [0u8; 32];
                                blind_bytes.copy_from_slice(blind.as_bytes());
                                current_auction.verified_blinds.insert(local_peer_id.to_string(), blind_bytes);
                                has_revealed = true;
                            }
                        }
                    }

                    if current_auction.verdict_received && !executed_auctions.contains(auction_id) {
                        local_resolution_queue.push(auction_id.clone());
                        executed_auctions.insert(auction_id.clone());
                    }

                    if current_unix_secs > current_auction.reveal_deadline && !current_auction.verdict_received {
                        if current_auction.validator_id.as_ref() == Some(&local_peer_id.to_string()) && !current_auction.resolved {
                            current_auction.resolve(); 
                            let msg = network::NetworkMessage::Verdict {
                                auction_id: auction_id.clone(),
                                validator_id: local_peer_id.to_string(),
                                winner_id: current_auction.winner_id.clone(),
                                clearing_price: current_auction.clearing_price,
                                slash_list: current_auction.slash_list.clone(),
                            };
                            let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), serde_json::to_string(&msg).unwrap().as_bytes());
                            current_auction.verdict_received = true; 
                        }
                        if current_auction.validator_id.is_none() || current_unix_secs > current_auction.reveal_deadline + 15 {
                            if !current_auction.resolved {
                                current_auction.resolve();
                                local_resolution_queue.push(auction_id.clone()); 
                                executed_auctions.insert(auction_id.clone());
                            }
                        }
                    }
                }

                for auction_id in local_resolution_queue {
                    if let Some(auction_to_close) = state.active_auctions.get_mut(&auction_id) {
                        let my_id = local_peer_id.to_string();
                        let stake_amount = auction_to_close.reserve_price * 2;
                        let successful_referee = auction_to_close.validator_id.is_some() && auction_to_close.verdict_received;
                        let validator_fee = if successful_referee { 5 } else { 0 };
                        auction_to_close.verdict_received = true; 
                        has_revealed = false; 

                        if Some(my_id.clone()) == auction_to_close.validator_id {
                            state.my_locked_credits -= 100; 
                            if successful_referee {
                                state.my_credits += 100 + validator_fee; 
                            } else {
                                state.my_credits += 100; 
                            }
                        }

                        if auction_to_close.slash_list.contains(&my_id) {
                            if let Some(my_bid) = state.my_secret_bid {
                                state.my_locked_credits -= my_bid; 
                            }
                        }
                        
                        if auction_to_close.failed {
                            if auction_to_close.seller_id == my_id {
                                state.my_locked_credits -= stake_amount;
                                state.my_credits += stake_amount;
                                state.my_locked_energy -= auction_to_close.energy_amount;
                                state.my_energy += auction_to_close.energy_amount;
                            }
                            if let Some(my_bid) = auction_to_close.verified_bids.get(&my_id) {
                                state.my_locked_credits -= my_bid;
                                state.my_credits += my_bid;
                            }
                        } else if !auction_to_close.slash_list.contains(&my_id) { 
                            let winner = auction_to_close.winner_id.as_ref().unwrap();
                            let price = auction_to_close.clearing_price;
                            
                            if auction_to_close.seller_id == my_id {
                                state.my_locked_credits -= stake_amount;
                                state.my_credits += stake_amount + price - validator_fee; 
                                state.my_locked_energy -= auction_to_close.energy_amount; 
                            }
                            
                            if winner == &my_id {
                                let my_bid = auction_to_close.verified_bids.get(&my_id).unwrap();
                                state.my_locked_credits -= my_bid;
                                state.my_credits += my_bid - price; 
                                state.my_energy += auction_to_close.energy_amount; 
                            } else if let Some(my_bid) = auction_to_close.verified_bids.get(&my_id) {
                                state.my_locked_credits -= my_bid;
                                state.my_credits += my_bid;
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
                    println!("   Credits: {} (Locked: {})", state.my_credits, state.my_locked_credits);
                    println!("   Battery: {} kWh (Locked: {} kWh)\n", state.my_energy, state.my_locked_energy);
                }
                else if line_str.starts_with("SELL ") && parts.len() == 3 {
                    if let (Ok(energy_amount), Ok(reserve_price)) = (parts[1].parse::<u64>(), parts[2].parse::<u64>()) {
                        let required_stake = reserve_price * 2;
                        if state.my_credits < required_stake {
                            println!("❌ Error: Need {} credits.", required_stake);
                        } else {
                            state.my_credits -= required_stake;
                            state.my_locked_credits += required_stake;
                            state.my_energy -= energy_amount;
                            state.my_locked_energy += energy_amount;
                            let current_unix_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                            let new_auction_id = format!("AUC_{}", current_unix_secs); 
                            println!("📢 AUCTION LIVE! Your unique Auction ID is: {}", new_auction_id);
                            let new_auction = auction::Auction::new(new_auction_id.clone(), local_peer_id.to_string(), energy_amount, reserve_price);
                            state.active_auctions.insert(new_auction_id.clone(), new_auction);
                            let msg = network::NetworkMessage::AnnounceAuction {
                                auction_id: new_auction_id, seller_id: local_peer_id.to_string(), energy_amount, reserve_price,
                            };
                            let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), serde_json::to_string(&msg).unwrap().as_bytes());
                        }
                    }
                } 
                else if line_str == "LOBBY" {
                    println!("\n🏛️  GLOBAL MARKETPLACE LOBBY 🏛️");
                    let current_unix_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                    
                    for (id, a) in &state.active_auctions {
                        let status = if a.verdict_received { "CLOSED".to_string() } 
                        else if current_unix_secs < a.commit_deadline { format!("🟢 COMMIT ({}s)", a.commit_deadline - current_unix_secs) } 
                        else if current_unix_secs < a.reveal_deadline { format!("🟡 REVEAL ({}s)", a.reveal_deadline - current_unix_secs) } 
                        else { "🔴 RESOLVING".to_string() };
                        
                        let val_status = if a.validator_id.is_some() { "🛡️ Guarded" } else { "⚠️ Unvalidated" };
                        println!("   ID: {} | {} | {} kWh | Reserve: {} | {}", id, val_status, a.energy_amount, a.reserve_price, status);
                    }
                } 
                else if line_str.starts_with("VALIDATE ") && parts.len() == 2 {
                    let target_id = parts[1].to_string();
                    if !state.active_auctions.contains_key(&target_id) {
                        println!("❌ Error: Auction {} not found.", target_id);
                    } else if state.my_credits < 100 {
                        println!("❌ Error: Need 100 credits for Honesty Bond.");
                    } else if let Some(target_auction) = state.active_auctions.get_mut(&target_id) {
                        if target_auction.validator_id.is_none() {
                            state.my_credits -= 100;
                            state.my_locked_credits += 100;
                            println!("🔒 Locked 100 credits. Applying to be Network Referee...");
                            target_auction.validator_id = Some(local_peer_id.to_string());
                            let msg = network::NetworkMessage::IntentToValidate {
                                auction_id: target_id.clone(), validator_id: local_peer_id.to_string(),
                            };
                            let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), serde_json::to_string(&msg).unwrap().as_bytes());
                        }
                    }
                }
                else if line_str.starts_with("JOIN ") && parts.len() == 2 {
                    let target_id = parts[1].to_string();
                    if state.active_auctions.contains_key(&target_id) {
                        state.current_joined_auction = Some(target_id.clone());
                        println!("✅ Joined auction {}.", target_id);
                    }
                }
                else if line_str.starts_with("BID ") && parts.len() == 2 {
                    if let (Some(joined_id), Ok(bid_amount)) = (&state.current_joined_auction, parts[1].parse::<u64>()) {
                        if state.my_credits < bid_amount {
                            println!("❌ Error: Insufficient funds.");
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
                            println!("🔒 Locked {} credits. Sent Binding Hash!", bid_amount);
                        }
                    }
                }
            }

            event = swarm.select_next_some() => match event {
                SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                    let addr = endpoint.get_remote_address();
                    println!("🔌 [TCP HANDSHAKE]: Connected to peer {} via [{}]", &peer_id.to_string()[0..8], addr);
                    
                    if peer_id.to_string() == RELAY_PEER_ID {
                        println!("📡 [RELAY]: Securely attached to Cloud Relay.");
                        swarm.behaviour_mut().kad.add_address(&peer_id, addr.clone());
                        
                        match swarm.behaviour_mut().kad.bootstrap() {
                            Ok(_) => println!("🔍 [DHT]: Bootstrap initiated. Mapping the local database..."),
                            Err(e) => println!("⚠️ [DHT ERROR]: Bootstrap failed: {:?}", e),
                        }

                        match swarm.behaviour_mut().kad.start_providing(room_key.clone()) {
                            Ok(_) => println!("📢 [DHT]: Telling the Cloud Relay we are officially open for auctions..."),
                            Err(e) => println!("⚠️ [DHT ERROR]: Failed to announce presence: {:?}", e),
                        }

                        if let Some(relay_addr) = &global_relay_addr {
                            let relay_listen_addr = relay_addr.clone().with(Protocol::P2pCircuit);
                            match swarm.listen_on(relay_listen_addr) {
                                Ok(_) => println!("⏳ [CIRCUIT]: Listening for incoming hole-punches on relay..."),
                                Err(e) => println!("❌ [CIRCUIT ERROR]: Failed to listen on relay: {:?}", e),
                            }
                        }
                    } else {
                        println!("✅ [GOSSIP MESH]: Direct peer connection established! Adding {} to auction room.", &peer_id.to_string()[0..8]);
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                    }
                },
                
                SwarmEvent::Dialing { peer_id, .. } => {
                    if let Some(peer) = peer_id {
                        println!("📞 [DIALER]: Swarm is actively attempting to dial {}...", &peer.to_string()[0..8]);
                    }
                },

                SwarmEvent::OutgoingConnectionError { peer_id, error, .. } => {
                    if let Some(peer) = peer_id {
                        println!("💥 [CONNECTION FAILED]: Could not reach peer {}. Reason: {:?}", &peer.to_string()[0..8], error);
                    }
                },

                SwarmEvent::NewListenAddr { address, .. } => {
                    if address.to_string().contains("p2p-circuit") {
                        println!("✅ [CIRCUIT]: Successfully secured our routing slot on the Cloud Relay!");
                    }
                },

                // 🔥 NEW: Handle UPnP Events to let you know if Port Forwarding succeeded
                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Upnp(event)) => {
                    match event {
                        libp2p::upnp::Event::NewExternalAddr(addr) => println!("🌐 [UPnP]: Router automatically opened port! Public Address: {}", addr),
                        libp2p::upnp::Event::GatewayNotFound => println!("⚠️ [UPnP]: No UPnP-enabled router found on local network."),
                        libp2p::upnp::Event::NonRoutableGateway => println!("⚠️ [UPnP]: Router is behind a Carrier-Grade NAT. Direct dial might fail."),
                        _ => {}
                    }
                },
                
                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Kad(event)) => {
                    match event {
                        libp2p::kad::Event::OutboundQueryProgressed { result, .. } => {
                            match result {
                                libp2p::kad::QueryResult::StartProviding(Ok(_)) => {
                                    println!("✅ [DHT SYSTEM]: Successfully registered as an Active Node on the Cloud Relay.");
                                },
                                libp2p::kad::QueryResult::GetProviders(Ok(ok)) => {
                                    match ok {
                                        libp2p::kad::GetProvidersOk::FoundProviders { providers, .. } => {
                                            for peer in providers {
                                                let peer_str = peer.to_string();
                                                
                                                if peer_str != RELAY_PEER_ID && peer_str != local_peer_id.to_string() {
                                                    
                                                    if known_peers.insert(peer_str.clone()) {
                                                        println!("📡 [RADAR SUCCESS]: Discovered Peer {}. Executing Hole-Punch...", &peer_str[0..8]);
                                                        
                                                        if let Some(relay_addr) = &global_relay_addr {
                                                            let circuit_addr = relay_addr.clone()
                                                                .with(Protocol::P2pCircuit)
                                                                .with(Protocol::P2p(peer));
                                                            
                                                            match swarm.dial(circuit_addr) {
                                                                Ok(_) => println!("🚀 [DIALER]: Dial command submitted to Swarm Successfully."),
                                                                Err(e) => println!("❌ [DIALER FATAL]: Swarm completely rejected the dial command: {:?}", e),
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        },
                                        libp2p::kad::GetProvidersOk::FinishedWithNoAdditionalRecord { .. } => {}
                                    }
                                },
                                _ => {}
                            }
                        },
                        _ => {}
                    }
                },

                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Dcutr(event)) => {
                    println!("🕳️ [DCUtR]: Hole-Punching Event triggered: {:?}", event);
                },
                
                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Gossipsub(libp2p::gossipsub::Event::Message { message, .. })) => {
                    if let Ok(parsed_msg) = serde_json::from_str::<network::NetworkMessage>(&String::from_utf8_lossy(&message.data)) {
                        match parsed_msg {
                            network::NetworkMessage::AnnounceAuction { auction_id, seller_id, energy_amount, reserve_price } => {
                                if !state.active_auctions.contains_key(&auction_id) {
                                    println!("📢 NEW MARKET: {}... selling {} kWh (Reserve: {}, ID: {})", &seller_id[0..8], energy_amount, reserve_price, auction_id);
                                    let new_auction = auction::Auction::new(auction_id.clone(), seller_id, energy_amount, reserve_price);
                                    state.active_auctions.insert(auction_id, new_auction);
                                }
                            },
                            network::NetworkMessage::IntentToValidate { auction_id, validator_id } => {
                                if let Some(target_auction) = state.active_auctions.get_mut(&auction_id) {
                                    if target_auction.validator_id.is_none() {
                                        println!("🛡️ Network: Peer {} was hired as the official Referee for {}!", &validator_id[0..8], auction_id);
                                        target_auction.validator_id = Some(validator_id);
                                    }
                                }
                            },
                            network::NetworkMessage::Verdict { auction_id, .. } => {
                                if let Some(target_auction) = state.active_auctions.get_mut(&auction_id) {
                                    if target_auction.verdict_received { continue; }
                                    target_auction.resolve(); 
                                    target_auction.verdict_received = true; 
                                }
                            },
                            network::NetworkMessage::Commit { auction_id, bidder_id, binding_hash } => {
                                if let Some(target_auction) = state.active_auctions.get_mut(&auction_id) {
                                    if !target_auction.received_commitments.contains_key(&bidder_id) {
                                        println!("📥 Network: Received hidden cryptographic bid from {}...", &bidder_id[0..8]);
                                        target_auction.received_commitments.insert(bidder_id, binding_hash);
                                    }
                                }
                            },
                            network::NetworkMessage::Reveal { auction_id, bidder_id, bid, blind_hex } => {
                                if let Some(target_auction) = state.active_auctions.get_mut(&auction_id) {
                                    if let Some(stored_binding_hash) = target_auction.received_commitments.get(&bidder_id) {
                                        if crypto::verify_binding_hash(stored_binding_hash, bid, &blind_hex, &bidder_id) {
                                            target_auction.verified_bids.insert(bidder_id.clone(), bid);
                                            println!("👀 Network: Peer {} revealed their bid of {} credits!", &bidder_id[0..8], bid);
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