mod crypto;
mod network;
mod auction;

use curve25519_dalek::scalar::Scalar;
use rand::thread_rng;
use std::time::{SystemTime, UNIX_EPOCH};
use std::error::Error;
use tokio::{io, io::AsyncBufReadExt, select};
use futures::StreamExt; 
use libp2p::swarm::SwarmEvent;

const CURRENT_AUCTION_ID: &str = "ENERGY_AUCTION_001"; 
const MAX_MESSAGE_AGE_SECS: u64 = 300; 

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("--- Starting Decentralized Energy Smart Contract ---");
    println!("🛡️ Active Defenses: Ed25519 Signatures, Replay Protection, Spam Mitigation");

    let id_keys = libp2p::identity::Keypair::generate_ed25519();
    let local_peer_id = id_keys.public().to_peer_id();
    println!("My Peer ID: {}", local_peer_id);

    let mut swarm = network::setup_swarm(id_keys, local_peer_id)?;
    swarm.listen_on("/ip4/127.0.0.1/tcp/0".parse()?)?;

    println!("---------------------------------------------------------");
    println!("COMMANDS: BID <number>, REVEAL, RESOLVE");
    println!("---------------------------------------------------------");

    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let mut state = auction::AuctionState::new();
    let topic = libp2p::gossipsub::IdentTopic::new("energy-auction");

    loop {
        select! {
            // EVENT A: Terminal Commands
            Ok(Some(line)) = stdin.next_line() => {
                let line_str = line.trim(); 
                
                if line_str.starts_with("BID ") {
                    let parts: Vec<&str> = line_str.split_whitespace().collect();
                    if parts.len() == 2 {
                        if let Ok(bid_amount) = parts[1].parse::<u64>() {
                            let mut rng = thread_rng();
                            let r = Scalar::random(&mut rng);
                            
                            state.my_secret_bid = Some(bid_amount);
                            state.my_secret_blind = Some(r);
                            
                            let my_commitment = crypto::commit(bid_amount, r);
                            let commitment_hex = hex::encode(my_commitment.compress().as_bytes());
                            let current_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                            
                            let msg = network::NetworkMessage::Commit {
                                auction_id: CURRENT_AUCTION_ID.to_string(),
                                timestamp: current_time,
                                bidder_id: local_peer_id.to_string(),
                                commitment: commitment_hex,
                            };
                            
                            let json_payload = serde_json::to_string(&msg)?;
                            let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), json_payload.as_bytes());
                            println!("🔒 Sent locked bid for {} credits!", bid_amount);
                        }
                    }
                } else if line_str == "REVEAL" {
                    if let (Some(bid), Some(blind)) = (state.my_secret_bid, state.my_secret_blind) {
                        let blind_hex = hex::encode(blind.as_bytes());
                        let current_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

                        let msg = network::NetworkMessage::Reveal {
                            auction_id: CURRENT_AUCTION_ID.to_string(),
                            timestamp: current_time,
                            bidder_id: local_peer_id.to_string(),
                            bid,
                            blind_hex,
                        };
                        
                        state.verified_bids.insert(local_peer_id.to_string(), bid);
                        let json_payload = serde_json::to_string(&msg)?;
                        let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), json_payload.as_bytes());
                        println!("🔓 Revealed secret key to the network!");
                    }
                } else if line_str == "RESOLVE" {
                    state.resolve();
                }
            }

            // EVENT B: Network Traffic
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Mdns(libp2p::mdns::Event::Discovered(list))) => {
                    for (peer_id, multiaddr) in list {
                        use libp2p::multiaddr::Protocol;
                        let mut local_addr = libp2p::Multiaddr::empty();
                        for p in multiaddr.iter() {
                            match p {
                                Protocol::Ip4(_) => local_addr.push(Protocol::Ip4("127.0.0.1".parse().unwrap())),
                                _ => local_addr.push(p),
                            }
                        }
                        if local_peer_id > peer_id {
                            let _ = swarm.dial(local_addr);
                        } 
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                    }
                },
                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Gossipsub(libp2p::gossipsub::Event::Subscribed { peer_id, .. })) => {
                    println!("✅ Neighbor {} joined!", peer_id);
                },
                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Gossipsub(libp2p::gossipsub::Event::Message { message, .. })) => {
                    if let Ok(parsed_msg) = serde_json::from_str::<network::NetworkMessage>(&String::from_utf8_lossy(&message.data)) {
                        let current_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

                        match parsed_msg {
                            network::NetworkMessage::Commit { auction_id, timestamp, bidder_id, commitment } => {
                                if auction_id == CURRENT_AUCTION_ID && current_time <= timestamp + MAX_MESSAGE_AGE_SECS {
                                    if !state.received_commitments.contains_key(&bidder_id) {
                                        println!("📥 Received Valid Locked Bid from {}", bidder_id);
                                        state.received_commitments.insert(bidder_id, commitment);
                                    }
                                }
                            },
                            network::NetworkMessage::Reveal { auction_id, timestamp, bidder_id, bid, blind_hex } => {
                                if auction_id == CURRENT_AUCTION_ID && current_time <= timestamp + MAX_MESSAGE_AGE_SECS {
                                    if !state.verified_bids.contains_key(&bidder_id) {
                                        if let Some(stored_hex) = state.received_commitments.get(&bidder_id) {
                                            if crypto::verify_commitment(stored_hex, bid, &blind_hex) {
                                                println!("    ✅ VERIFICATION PASSED: Adding {} to final ledger!", bidder_id);
                                                state.verified_bids.insert(bidder_id, bid);
                                            } else {
                                                println!("    ❌ VERIFICATION FAILED: Cheating detected!");
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                _ => {}
            }
        }
    }
}