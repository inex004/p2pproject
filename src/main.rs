mod crypto;
mod network;
mod auction;

use std::fs;
use curve25519_dalek::scalar::Scalar;
use rand::thread_rng;
use std::error::Error;
use tokio::{io, io::AsyncBufReadExt, select};
use futures::StreamExt; 
use libp2p::swarm::SwarmEvent;
use std::env;

const CURRENT_AUCTION_ID: &str = "ENERGY_AUCTION_001"; 

const AUTHORIZED_METERS: [&str; 4] = [
    "12D3KooWP12edPP1guWsgxgmr74Lt1aE7JwFksyCiew9Srr8RjwB", // Node A
    "12D3KooWFuoRX7BQ9PJHUxzvJjzuJFx11TYPWX6A3pRWqUHxZZeg", // Node B
    "12D3KooWCaSszh4dejZ2zWaRUfeXadBmsrvXmRZDokghX2pCSUf9", // Node C
    "12D3KooWMvGUTxq75wzFwMy7YjKaKigV4w2UxqcjTbNvUDaxXWHs", // Node D
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
    
    // Elevate the node to act as a public Kademlia phonebook server
    swarm.behaviour_mut().kademlia.set_mode(Some(libp2p::kad::Mode::Server));
    println!("📡 Listening for neighbors on: {}", listen_addr);

    if args.len() > 2 {
        let bootstrap_addr: libp2p::Multiaddr = args[2].parse().expect("Invalid bootstrap multiaddress");
        println!("🔗 Bootstrapping... Dialing known neighbor: {}", bootstrap_addr);
        swarm.dial(bootstrap_addr)?;
    }

    println!("---------------------------------------------------------");
    println!("COMMANDS: BID <number>, REVEAL, RESOLVE, SYNC, HEAL");
    println!("---------------------------------------------------------");

    let mut stdin = io::BufReader::new(io::stdin()).lines();
    let mut state = auction::AuctionState::new();
    let topic = libp2p::gossipsub::IdentTopic::new("energy-auction");

    loop {
        select! {
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
                            let current_clock = state.tick();
                            
                            let msg = network::NetworkMessage::Commit {
                                auction_id: CURRENT_AUCTION_ID.to_string(),
                                lamport_clock: current_clock,
                                bidder_id: local_peer_id.to_string(),
                                commitment: commitment_hex,
                            };
                            
                            let json_payload = serde_json::to_string(&msg)?;
                            let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), json_payload.as_bytes());
                            println!("🔒 Sent locked bid for {} credits! [Lamport Time: {}]", bid_amount, current_clock);
                        }
                    }
                } else if line_str == "REVEAL" {
                    if let (Some(bid), Some(blind)) = (state.my_secret_bid, state.my_secret_blind) {
                        let blind_hex = hex::encode(blind.as_bytes());
                        let current_clock = state.tick();

                        let msg = network::NetworkMessage::Reveal {
                            auction_id: CURRENT_AUCTION_ID.to_string(),
                            lamport_clock: current_clock,
                            bidder_id: local_peer_id.to_string(),
                            bid,
                            blind_hex,
                        };
                        
                        state.verified_bids.insert(local_peer_id.to_string(), bid);
                        let json_payload = serde_json::to_string(&msg)?;
                        let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), json_payload.as_bytes());
                        println!("🔓 Revealed secret key to the network! [Lamport Time: {}]", current_clock);
                    }
                } else if line_str == "RESOLVE" {
                    state.resolve();
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

                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Identify(event)) => {
                    if let libp2p::identify::Event::Received { peer_id, info, .. } = event {
                        for addr in info.listen_addrs {
                            swarm.behaviour_mut().kademlia.add_address(&peer_id, addr.clone());
                        }
                    }
                },

                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Kademlia(event)) => {
                    if let libp2p::kad::Event::RoutingUpdated { peer: _, .. } = event {
                    }
                },

                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Gossipsub(libp2p::gossipsub::Event::Subscribed { peer_id, .. })) => {
                    println!("✅ Neighbor {} subscribed to the auction topic!", peer_id);
                },
                
                // --- ARMED FIREWALL: CATCH AND PUNISH ---
                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Gossipsub(libp2p::gossipsub::Event::Message { message, propagation_source, message_id })) => {
                    if let Ok(parsed_msg) = serde_json::from_str::<network::NetworkMessage>(&String::from_utf8_lossy(&message.data)) {
                        
                        let incoming_peer_id = match &parsed_msg {
                            network::NetworkMessage::Commit { bidder_id, .. } => bidder_id,
                            network::NetworkMessage::Reveal { bidder_id, .. } => bidder_id,
                        };

                        if !AUTHORIZED_METERS.contains(&incoming_peer_id.as_str()) {
                            println!("🚨 SECURITY ALERT: Unauthorized peer {} blocked! Docking reputation score...", incoming_peer_id);
                            
                            // PULL THE TRIGGER: Tell Gossipsub to reject the message and penalize the sender
                            let _ = swarm.behaviour_mut().gossipsub.report_message_validation_result(
                                &message_id, 
                                &propagation_source, 
                                libp2p::gossipsub::MessageAcceptance::Reject
                            );
                            continue; 
                        }

                        let incoming_clock = match &parsed_msg {
                            network::NetworkMessage::Commit { lamport_clock, .. } => *lamport_clock,
                            network::NetworkMessage::Reveal { lamport_clock, .. } => *lamport_clock,
                        };
                        state.sync_clock(incoming_clock);

                        match parsed_msg {
                            network::NetworkMessage::Commit { auction_id, bidder_id, commitment, .. } => {
                                if auction_id == CURRENT_AUCTION_ID {
                                    if !state.received_commitments.contains_key(&bidder_id) {
                                        println!("📥 Received Valid Locked Bid from {} [Lamport Time: {}]", bidder_id, state.lamport_clock);
                                        state.received_commitments.insert(bidder_id, commitment);
                                    }
                                }
                            },
                            network::NetworkMessage::Reveal { auction_id, bidder_id, bid, blind_hex, .. } => {
                                if auction_id == CURRENT_AUCTION_ID {
                                    if !state.verified_bids.contains_key(&bidder_id) {
                                        if let Some(stored_hex) = state.received_commitments.get(&bidder_id) {
                                            if crypto::verify_commitment(stored_hex, bid, &blind_hex) {
                                                println!("    ✅ VERIFICATION PASSED: Adding {} to final ledger! [Lamport Time: {}]", bidder_id, state.lamport_clock);
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