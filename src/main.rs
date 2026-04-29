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
use std::time::{SystemTime, UNIX_EPOCH, Duration}; 

const CURRENT_AUCTION_ID: &str = "ENERGY_AUCTION_001"; 
// 🔥 NEW: 3-second absolute threshold to prevent Timestamp Spoofing / Time-Jacking
const MAX_NETWORK_DELAY: u64 = 3000; 
const REVEAL_TIMEOUT: u64 = 60; 

const AUTHORIZED_METERS: [&str; 4] = [
    "12D3KooWP12edPP1guWsgxgmr74Lt1aE7JwFksyCiew9Srr8RjwB", 
    "12D3KooWFuoRX7BQ9PJHUxzvJjzuJFx11TYPWX6A3pRWqUHxZZeg", 
    "12D3KooWCaSszh4dejZ2zWaRUfeXadBmsrvXmRZDokghX2pCSUf9", 
    "12D3KooWMvGUTxq75wzFwMy7YjKaKigV4w2UxqcjTbNvUDaxXWHs", 
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("⚠️ Usage: cargo run -- <listen_port> [bootstrap_address]");
        std::process::exit(1);
    }

    let listen_port: u16 = args[1].parse().expect("Invalid port number");

    println!("--- Starting Real-World P2P Energy Node ---");
    
    let key_file = format!("meter_{}.key", listen_port);
    let id_keys = if let Ok(bytes) = fs::read(&key_file) {
        libp2p::identity::Keypair::from_protobuf_encoding(&bytes).unwrap()
    } else {
        let new_key = libp2p::identity::Keypair::generate_ed25519();
        fs::write(&key_file, new_key.to_protobuf_encoding().unwrap()).unwrap();
        new_key
    };

    let local_peer_id = id_keys.public().to_peer_id();
    println!("My Permanent Peer ID: {}", local_peer_id);

    let mut swarm = network::setup_swarm(id_keys, local_peer_id)?;
    
    let listen_addr: libp2p::Multiaddr = format!("/ip4/127.0.0.1/tcp/{}", listen_port).parse()?;
    swarm.listen_on(listen_addr.clone())?;
    
    swarm.behaviour_mut().kademlia.set_mode(Some(libp2p::kad::Mode::Server));
    println!("📡 Listening for neighbors on: {}", listen_addr);

    if args.len() > 2 {
        let bootstrap_addr: libp2p::Multiaddr = args[2].parse().expect("Invalid bootstrap multiaddress");
        println!("🔗 Bootstrapping... Dialing known neighbor: {}", bootstrap_addr);
        swarm.dial(bootstrap_addr)?;
    }

    println!("---------------------------------------------------------");
    println!("COMMANDS: BID <number>, QUEUE, TIME, SLASH <peer_id>, SYNC, HEAL"); 
    println!("---------------------------------------------------------");

    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let mut state = auction::AuctionState::new();
    let topic = libp2p::gossipsub::IdentTopic::new("energy-auction");

    let mut background_timer = time::interval(Duration::from_secs(1));
    let mut has_resolved = false;
    
    let mut current_expected_peer: Option<String> = None;
    let mut expected_peer_start_time: u64 = 0;

    loop {
        select! {
            _ = background_timer.tick() => {
                let current_unix_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

                if current_unix_secs > state.commit_deadline && !has_resolved {
                    
                    let mut unrevealed: Vec<(&String, &u64)> = state.received_commitments.iter()
                        .filter(|(id, _)| !state.verified_bids.contains_key(*id))
                        .map(|(id, (_, timestamp))| (id, timestamp))
                        .collect();
                    
                    // 🔥 NEW: Total Order Sorting (Primary: Timestamp DESC, Secondary: PeerID DESC)
                    unrevealed.sort_by(|a, b| match b.1.cmp(a.1) {
                        std::cmp::Ordering::Equal => b.0.cmp(a.0),
                        other => other,
                    });

                    if unrevealed.is_empty() && !state.received_commitments.is_empty() {
                        println!("🏁 All peers have revealed in strict LIFO order! Automatically resolving auction...");
                        state.resolve();
                        has_resolved = true;
                        current_expected_peer = None; 
                    } 
                    else if let Some((expected_peer, _)) = unrevealed.first() {
                        let expected_peer_cloned = expected_peer.to_string();

                        if Some(&expected_peer_cloned) != current_expected_peer.as_ref() {
                            current_expected_peer = Some(expected_peer_cloned.clone());
                            expected_peer_start_time = current_unix_secs;
                        } else if current_unix_secs.saturating_sub(expected_peer_start_time) >= REVEAL_TIMEOUT {
                            println!("\n⚖️ ======================================================================= ⚖️");
                            println!("⚖️ AUTOMATED SLASH EXECUTED: Peer {} failed to reveal in 60s! ⚖️", expected_peer_cloned);
                            println!("⚖️ ======================================================================= ⚖️\n");
                            
                            state.received_commitments.remove(&expected_peer_cloned);
                            current_expected_peer = None; 
                            continue; 
                        }

                        let my_id_string = local_peer_id.to_string();
                        
                        if expected_peer_cloned == my_id_string && !state.verified_bids.contains_key(&my_id_string) {
                            if let (Some(bid), Some(blind)) = (state.my_secret_bid, state.my_secret_blind) {
                                println!("🤖 AUTOMATION: I am Next-In-Line! Broadcasting Reveal payload for Peer ID: {}...", local_peer_id);
                                
                                let blind_hex = hex::encode(blind.as_bytes());
                                let current_timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;

                                let msg = network::NetworkMessage::Reveal {
                                    auction_id: CURRENT_AUCTION_ID.to_string(),
                                    timestamp: current_timestamp,
                                    bidder_id: local_peer_id.to_string(),
                                    bid,
                                    blind_hex,
                                };
                                
                                state.verified_bids.insert(local_peer_id.to_string(), bid);
                                let json_payload = serde_json::to_string(&msg).unwrap();
                                let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), json_payload.as_bytes());
                                println!("🔓 Revealed secret key to the network! [Physical Time (ms): {}]", current_timestamp);
                            }
                        }
                    }
                }
            }

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
                            
                            // 🔥 NEW: Millisecond precision physical timestamp
                            let current_timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
                            
                            state.received_commitments.insert(local_peer_id.to_string(), (commitment_hex.clone(), current_timestamp));
                            
                            let msg = network::NetworkMessage::Commit {
                                auction_id: CURRENT_AUCTION_ID.to_string(),
                                timestamp: current_timestamp,
                                bidder_id: local_peer_id.to_string(),
                                commitment: commitment_hex,
                            };
                            
                            let json_payload = serde_json::to_string(&msg).unwrap();
                            let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), json_payload.as_bytes());
                            println!("🔒 Sent locked bid for {} credits! [Physical Time (ms): {}]", bid_amount, current_timestamp);
                        }
                    }
                } else if line_str.starts_with("SLASH ") {
                    let parts: Vec<&str> = line_str.split_whitespace().collect();
                    if parts.len() == 2 {
                        let target = parts[1];
                        if state.received_commitments.remove(target).is_some() {
                            println!("\n⚔️ ======================================================================= ⚔️");
                            println!("⚔️ SLASH EXECUTED: Peer {} kicked for stalling! ⚔️", target);
                            println!("⚔️ ======================================================================= ⚔️\n");
                        } else {
                            println!("⚠️ Peer not found in queue.");
                        }
                    }
                } else if line_str == "TIME" {
                    let current_unix_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                    
                    println!("\n---------------------------------------------------------");
                    if current_unix_secs < state.commit_deadline {
                        let remaining = state.commit_deadline - current_unix_secs;
                        let elapsed = 300 - remaining; 
                        
                        println!("⏱️  TIME ELAPSED:   {} seconds", elapsed);
                        println!("⏳ TIME REMAINING: {} seconds", remaining);
                    } else {
                        println!("⏰ COMMIT PHASE IS CLOSED.");
                        if has_resolved {
                            println!("🏁 Auction has already been resolved.");
                        } else if let Some(target) = &current_expected_peer {
                            let elapsed = current_unix_secs.saturating_sub(expected_peer_start_time);
                            let remaining = REVEAL_TIMEOUT.saturating_sub(elapsed);
                            println!("🤖 REVEAL PHASE: Waiting on {}...", target);
                            println!("⏳ SLASH COUNTDOWN: {} seconds remaining before penalty.", remaining);
                        }
                    }
                    println!("---------------------------------------------------------\n");
                } else if line_str == "QUEUE" {
                    let current_unix_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

                    println!("\n==========================================================================================");
                    println!("                          📋 CURRENT REVEAL QUEUE (TOTAL ORDER) 📋                        ");
                    
                    if current_unix_secs < state.commit_deadline {
                        let remaining = state.commit_deadline - current_unix_secs;
                        println!("                          ⏳ DEADLINE CLOSES IN: {} SECONDS ⏳                          ", remaining);
                    } else if !has_resolved {
                        if let Some(_) = &current_expected_peer {
                            let elapsed = current_unix_secs.saturating_sub(expected_peer_start_time);
                            let remaining = REVEAL_TIMEOUT.saturating_sub(elapsed);
                            println!("                         ⏳ SLASHING NODE #1 IN: {} SECONDS ⏳                          ", remaining);
                        }
                    } else {
                        println!("                          🏁 AUCTION SUCCESSFULLY RESOLVED 🏁                           ");
                    }
                    println!("==========================================================================================");
                    println!("{:<6} | {:<53} | {:<15} | {:<15}", "ORDER", "PEER ID", "PHYSICAL TIME", "STATUS");
                    println!("-------|-------------------------------------------------------|-----------------|-------------");
                    
                    let mut queue: Vec<(&String, &u64)> = state.received_commitments.iter()
                        .map(|(id, (_, timestamp))| (id, timestamp))
                        .collect();
                    
                    // 🔥 NEW: Total Order Sorting logic applied to the UI display as well
                    queue.sort_by(|a, b| match b.1.cmp(a.1) {
                        std::cmp::Ordering::Equal => b.0.cmp(a.0),
                        other => other,
                    });

                    if queue.is_empty() {
                        println!(" (No commitments received yet)");
                    } else {
                        for (i, (peer_id, timestamp)) in queue.iter().enumerate() {
                            let status = if state.verified_bids.contains_key(*peer_id) {
                                "✅ REVEALED"
                            } else {
                                let is_next = queue[0..i].iter().all(|(id, _)| state.verified_bids.contains_key(*id));
                                if is_next {
                                    "👉 NEXT IN LINE"
                                } else {
                                    "⏳ WAITING..."
                                }
                            };
                            println!("{:<6} | {:<53} | {:<15} | {}", format!("#{}", i + 1), peer_id, timestamp, status);
                        }
                    }
                    println!("==========================================================================================\n");
                } else if line_str == "SYNC" {
                    println!("🔄 Synchronizing Kademlia DHT Phonebook across the network...");
                    let _ = swarm.behaviour_mut().kademlia.bootstrap();
                } else if line_str == "HEAL" {
                    println!("⚕️ Initiating Emergency Network Heal...");
                    let mut dial_count = 0;
                    let mut peers_to_dial = Vec::new();
                    
                    for bucket in swarm.behaviour_mut().kademlia.kbuckets() {
                        for entry in bucket.iter() {
                            let known_peer = entry.node.key.preimage().clone();
                            if known_peer != local_peer_id {
                                peers_to_dial.push(known_peer);
                            }
                        }
                    }

                    for peer in peers_to_dial {
                        println!("📞 Dialing known DHT peer: {}", peer);
                        let _ = swarm.dial(peer);
                        dial_count += 1;
                    }
                    
                    if dial_count == 0 {
                        println!("⚠️ Phonebook is empty. Cannot heal without a manual bootstrap.");
                    }
                }
            }

            event = swarm.select_next_some() => match event {
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    println!("🤝 TCP Connection established with peer: {}", peer_id);
                    swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                },

                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    println!("🔌 TCP CONNECTION SEVERED with peer: {}", peer_id);
                },

                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Identify(event)) => {
                    if let libp2p::identify::Event::Received { peer_id, info, .. } = event {
                        for addr in info.listen_addrs {
                            swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                        }
                    }
                },

                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Kademlia(event)) => {
                    if let libp2p::kad::Event::RoutingUpdated { peer, .. } = event {
                        println!("📖 KADEMLIA UPDATE: Added peer {} to the local phonebook!", peer);
                    }
                },

                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Gossipsub(libp2p::gossipsub::Event::Subscribed { peer_id, .. })) => {
                    println!("✅ Neighbor {} subscribed to the auction topic!", peer_id);
                },
                
                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Gossipsub(libp2p::gossipsub::Event::Message { message, propagation_source, message_id })) => {
                    if let Ok(parsed_msg) = serde_json::from_str::<network::NetworkMessage>(&String::from_utf8_lossy(&message.data)) {
                        
                        let incoming_peer_id = match &parsed_msg {
                            network::NetworkMessage::Commit { bidder_id, .. } => bidder_id,
                            network::NetworkMessage::Reveal { bidder_id, .. } => bidder_id,
                        };

                        if !AUTHORIZED_METERS.contains(&incoming_peer_id.as_str()) {
                            println!("🚨 SYBIL ALERT: Unauthorized peer {} blocked!", incoming_peer_id);
                            let _ = swarm.behaviour_mut().gossipsub.report_message_validation_result(&message_id, &propagation_source, libp2p::gossipsub::MessageAcceptance::Reject);
                            
                            if let Ok(peer) = incoming_peer_id.parse::<libp2p::PeerId>() {
                                println!("🔨 DROPPING HAMMER: Forcing disconnect on Hacker IP!");
                                let _ = swarm.disconnect_peer_id(peer);
                            }
                        }

                        let incoming_timestamp = match &parsed_msg {
                            network::NetworkMessage::Commit { timestamp, .. } => *timestamp,
                            network::NetworkMessage::Reveal { timestamp, .. } => *timestamp,
                        };

                        // 🔥 NEW: BOUNDED ACCEPTANCE WINDOW (Defeating Timestamp Spoofing)
                        let current_local_millis = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;

                        if current_local_millis.abs_diff(incoming_timestamp) > MAX_NETWORK_DELAY {
                            println!("🚨 TIME-JACKING ALERT: Peer {} reported a timestamp ({}ms) outside the 3-second network boundary! Local time is {}ms.", incoming_peer_id, incoming_timestamp, current_local_millis);
                            let _ = swarm.behaviour_mut().gossipsub.report_message_validation_result(&message_id, &propagation_source, libp2p::gossipsub::MessageAcceptance::Reject);
                            
                            if let Ok(peer) = incoming_peer_id.parse::<libp2p::PeerId>() {
                                println!("🔨 DROPPING HAMMER: Forcing disconnect on Rogue Node!");
                                let _ = swarm.disconnect_peer_id(peer);
                            }
                            continue;
                        }

                        match parsed_msg {
                            network::NetworkMessage::Commit { auction_id, bidder_id, commitment, .. } => {
                                if auction_id == CURRENT_AUCTION_ID {
                                    
                                    let current_unix_secs = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                                    
                                    if current_unix_secs > state.commit_deadline {
                                        println!("⏰ TIME'S UP! Rejected late bid from {}. Commit phase is closed!", bidder_id);
                                        continue; 
                                    }

                                    if !state.received_commitments.contains_key(&bidder_id) {
                                        println!("📥 Received Valid Locked Bid from {} [Physical Time (ms): {}]", bidder_id, incoming_timestamp);
                                        state.received_commitments.insert(bidder_id, (commitment, incoming_timestamp));
                                    }
                                }
                            },
                            network::NetworkMessage::Reveal { auction_id, bidder_id, bid, blind_hex, .. } => {
                                if auction_id == CURRENT_AUCTION_ID {
                                    if !state.verified_bids.contains_key(&bidder_id) {
                                        
                                        let mut unrevealed: Vec<(&String, &u64)> = state.received_commitments.iter()
                                            .filter(|(id, _)| !state.verified_bids.contains_key(*id))
                                            .map(|(id, (_, timestamp))| (id, timestamp))
                                            .collect();
                                        
                                        // 🔥 NEW: Total Order logic ensures no false-positive LIFO violations on concurrent bids
                                        unrevealed.sort_by(|a, b| match b.1.cmp(a.1) {
                                            std::cmp::Ordering::Equal => b.0.cmp(a.0),
                                            other => other,
                                        });

                                        if let Some((expected_peer, _expected_clock)) = unrevealed.first() {
                                            if &&bidder_id != expected_peer {
                                                println!("🚨 LIFO VIOLATION: {} tried to reveal out of turn!", bidder_id);
                                                println!("   Expected {} to reveal first. Dropping packet.", expected_peer);
                                                continue; 
                                            }
                                        }

                                        if let Some((stored_hex, _)) = state.received_commitments.get(&bidder_id) {
                                            if crypto::verify_commitment(stored_hex, bid, &blind_hex) {
                                                println!("    ✅ VERIFICATION PASSED: {} revealed in correct LIFO order!", bidder_id);
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