// --- CUSTOM MODULES ---
// We split the codebase into clean modules so this main file is just the orchestrator.
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

// --- GLOBAL CONSTANTS ---
// Hardcoded for this prototype. In a real system, this ID would change per auction.
const CURRENT_AUCTION_ID: &str = "ENERGY_AUCTION_001"; 
// 5-minute TTL (Time-To-Live) to give us enough time to test manually, 
// while still dropping old replay attacks.
const MAX_MESSAGE_AGE_SECS: u64 = 300; 

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("--- Starting Decentralized Energy Smart Contract ---");
    println!("🛡️ Active Defenses: Ed25519 Signatures, Replay Protection, Spam Mitigation");

    // 1. Generate our cryptographic identity for the network
    let id_keys = libp2p::identity::Keypair::generate_ed25519();
    let local_peer_id = id_keys.public().to_peer_id();
    println!("My Peer ID: {}", local_peer_id);

    // 2. Fire up the P2P networking stack
    let mut swarm = network::setup_swarm(id_keys, local_peer_id)?;
    swarm.listen_on("/ip4/127.0.0.1/tcp/0".parse()?)?;

    println!("---------------------------------------------------------");
    println!("COMMANDS: BID <number>, REVEAL, RESOLVE");
    println!("---------------------------------------------------------");

    // 3. Setup our state: User input reader, our ledgers, and the gossip topic
    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let mut state = auction::AuctionState::new();
    let topic = libp2p::gossipsub::IdentTopic::new("energy-auction");

    // 4. The Main Event Loop
    // We use tokio::select! to listen to BOTH the user typing and network traffic concurrently.
    loop {
        select! {
            // ==========================================
            // EVENT A: USER TYPED SOMETHING IN TERMINAL
            // ==========================================
            Ok(Some(line)) = stdin.next_line() => {
                let line_str = line.trim(); 
                
                if line_str.starts_with("BID ") {
                    // Extract the number from the command (e.g., "BID 100")
                    let parts: Vec<&str> = line_str.split_whitespace().collect();
                    if parts.len() == 2 {
                        if let Ok(bid_amount) = parts[1].parse::<u64>() {
                            
                            // Generate a random blinding factor (the secret key!)
                            let mut rng = thread_rng();
                            let r = Scalar::random(&mut rng);
                            
                            // Save our bid/secret in local memory so we can reveal it later
                            state.my_secret_bid = Some(bid_amount);
                            state.my_secret_blind = Some(r);
                            
                            // Do the math: lock the bid in a cryptographic envelope
                            let my_commitment = crypto::commit(bid_amount, r);
                            let commitment_hex = hex::encode(my_commitment.compress().as_bytes());
                            let current_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                            
                            // Package it into JSON and broadcast it to our neighbors
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
                    // Make sure we actually placed a bid first
                    if let (Some(bid), Some(blind)) = (state.my_secret_bid, state.my_secret_blind) {
                        let blind_hex = hex::encode(blind.as_bytes());
                        let current_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

                        // Broadcast our raw bid and the secret key to the network
                        let msg = network::NetworkMessage::Reveal {
                            auction_id: CURRENT_AUCTION_ID.to_string(),
                            timestamp: current_time,
                            bidder_id: local_peer_id.to_string(),
                            bid,
                            blind_hex,
                        };
                        
                        // Automatically trust our own bid and add it to our ledger
                        state.verified_bids.insert(local_peer_id.to_string(), bid);
                        
                        let json_payload = serde_json::to_string(&msg)?;
                        let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), json_payload.as_bytes());
                        println!("🔓 Revealed secret key to the network!");
                    }
                } else if line_str == "RESOLVE" {
                    // Trigger the smart contract to calculate the Vickrey winner
                    state.resolve();
                }
            }

            // ==========================================
            // EVENT B: NETWORK TRAFFIC ARRIVED
            // ==========================================
            event = swarm.select_next_some() => match event {
                
                // MDNS: Auto-discovery on the local network
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
                        // Dial the peer to establish a connection
                        if local_peer_id > peer_id {
                            let _ = swarm.dial(local_addr);
                        } 
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                    }
                },
                
                // GOSSIPSUB: A neighbor successfully connected to our topic
                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Gossipsub(libp2p::gossipsub::Event::Subscribed { peer_id, .. })) => {
                    println!("✅ Neighbor {} joined!", peer_id);
                },
                
                // GOSSIPSUB: We received a message from the mesh
                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Gossipsub(libp2p::gossipsub::Event::Message { message, .. })) => {
                    if let Ok(parsed_msg) = serde_json::from_str::<network::NetworkMessage>(&String::from_utf8_lossy(&message.data)) {
                        let current_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

                        match parsed_msg {
                            // Another node is submitting a sealed bid
                            network::NetworkMessage::Commit { auction_id, timestamp, bidder_id, commitment } => {
                                // Security Check: Is this a replay attack or expired?
                                if auction_id == CURRENT_AUCTION_ID && current_time <= timestamp + MAX_MESSAGE_AGE_SECS {
                                    // Security Check: One-Bid Rule (Spam defense)
                                    if !state.received_commitments.contains_key(&bidder_id) {
                                        println!("📥 Received Valid Locked Bid from {}", bidder_id);
                                        state.received_commitments.insert(bidder_id, commitment);
                                    }
                                }
                            },
                            
                            // Another node is publishing their key to prove their bid
                            network::NetworkMessage::Reveal { auction_id, timestamp, bidder_id, bid, blind_hex } => {
                                // Security Check: Replay/TTL
                                if auction_id == CURRENT_AUCTION_ID && current_time <= timestamp + MAX_MESSAGE_AGE_SECS {
                                    // Security Check: Did they already successfully reveal? (CPU exhaustion defense)
                                    if !state.verified_bids.contains_key(&bidder_id) {
                                        // Do we have their original sealed envelope?
                                        if let Some(stored_hex) = state.received_commitments.get(&bidder_id) {
                                            
                                            // THE ULTIMATE TEST: Does the curve math match the envelope?
                                            if crypto::verify_commitment(stored_hex, bid, &blind_hex) {
                                                println!("    ✅ VERIFICATION PASSED: Adding {} to final ledger!", bidder_id);
                                                // It's mathematically proven. Add to the official smart contract ledger.
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