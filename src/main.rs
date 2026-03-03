use curve25519_dalek::ristretto::RistrettoPoint;
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::constants::RISTRETTO_BASEPOINT_TABLE;
use rand::thread_rng;
use sha2::{Sha512, Digest};
use serde::{Serialize, Deserialize};
use std::collections::HashMap;

// Networking imports
use libp2p::{gossipsub, mdns, noise, swarm::{NetworkBehaviour, SwarmEvent}, tcp, yamux};
use std::error::Error;
use std::time::Duration;
use tokio::{io, io::AsyncBufReadExt, select};
use futures::StreamExt; 

// --- PHASE 1 & 2: MATH & DATA STRUCTURES ---

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum NetworkMessage {
    Commit { bidder_id: String, commitment: String },
    Reveal { bidder_id: String, bid: u64, blind_hex: String },
}

fn get_h_basepoint() -> RistrettoPoint {
    let mut hasher = Sha512::new();
    hasher.update(b"energy_auction_basepoint_h");
    let result = hasher.finalize(); 
    let bytes: [u8; 64] = result.into();
    RistrettoPoint::from_uniform_bytes(&bytes)
}

fn commit(bid_value: u64, blinding_factor: Scalar) -> RistrettoPoint {
    let g = &RISTRETTO_BASEPOINT_TABLE; 
    let h = get_h_basepoint();
    let v = Scalar::from(bid_value);
    (*g * &v) + (blinding_factor * h)
}

// --- PHASE 3 & 4: NETWORKING & AUCTION STATE ---

#[derive(NetworkBehaviour)]
struct AuctionNetworkBehaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("--- Starting Decentralized Energy Node (Phase 4) ---");

    let id_keys = libp2p::identity::Keypair::generate_ed25519();
    let local_peer_id = id_keys.public().to_peer_id();
    println!("My Peer ID: {}", local_peer_id);

    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(1))
        .validation_mode(gossipsub::ValidationMode::Strict)
        .build()
        .expect("Valid gossipsub config");

    let mut gossipsub_behaviour = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossipsub_config,
    ).expect("Valid gossipsub behaviour");

    let topic = gossipsub::IdentTopic::new("energy-auction");
    gossipsub_behaviour.subscribe(&topic)?;

    let mdns_behaviour = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?;

    let behaviour = AuctionNetworkBehaviour { gossipsub: gossipsub_behaviour, mdns: mdns_behaviour };

    let mut swarm = libp2p::SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
        .with_behaviour(|_| behaviour)?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    swarm.listen_on("/ip4/127.0.0.1/tcp/0".parse()?)?;

    println!("---------------------------------------------------------");
    println!("COMMANDS:");
    println!("Type 'BID <number>' to securely lock in a bid (e.g., BID 150).");
    println!("Type 'REVEAL' to publish your secret key and prove your bid.");
    println!("---------------------------------------------------------");

    let mut stdin = io::BufReader::new(io::stdin()).lines();

    let mut received_commitments: HashMap<String, String> = HashMap::new();
    let mut my_secret_bid: Option<u64> = None;
    let mut my_secret_blind: Option<Scalar> = None;

    loop {
        select! {
            // EVENT A: User types in the terminal
            Ok(Some(line)) = stdin.next_line() => {
                let line_str = line.trim(); 
                
                if line_str.starts_with("BID ") {
                    let parts: Vec<&str> = line_str.split_whitespace().collect();
                    if parts.len() == 2 {
                        if let Ok(bid_amount) = parts[1].parse::<u64>() {
                            let mut rng = thread_rng();
                            let r = Scalar::random(&mut rng);
                            
                            my_secret_bid = Some(bid_amount);
                            my_secret_blind = Some(r);
                            
                            let my_commitment = commit(bid_amount, r);
                            let commitment_hex = hex::encode(my_commitment.compress().as_bytes());
                            
                            let msg = NetworkMessage::Commit {
                                bidder_id: local_peer_id.to_string(),
                                commitment: commitment_hex.clone(),
                            };
                            let json_payload = serde_json::to_string(&msg)?;
                            
                            if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), json_payload.as_bytes()) {
                                println!("Failed to publish bid: {:?}", e);
                            } else {
                                println!("🔒 Sent locked bid for {}! (Commitment: {})", bid_amount, commitment_hex);
                            }
                        }
                    }
                } else if line_str == "REVEAL" {
                    if let (Some(bid), Some(blind)) = (my_secret_bid, my_secret_blind) {
                        let blind_hex = hex::encode(blind.as_bytes());
                        let msg = NetworkMessage::Reveal {
                            bidder_id: local_peer_id.to_string(),
                            bid,
                            blind_hex,
                        };
                        let json_payload = serde_json::to_string(&msg)?;
                        
                        if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), json_payload.as_bytes()) {
                            println!("Failed to publish reveal: {:?}", e);
                        } else {
                            println!("🔓 Revealed secret key to the network!");
                        }
                    } else {
                        println!("⚠️ You haven't placed a bid yet!");
                    }
                } else {
                    println!("Unknown command. Use 'BID <number>' or 'REVEAL'.");
                }
            }

            // EVENT B: Network event occurs
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(AuctionNetworkBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
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
                            if let Err(_) = swarm.dial(local_addr) {}
                        } 
                        swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                    }
                },
                SwarmEvent::Behaviour(AuctionNetworkBehaviourEvent::Gossipsub(gossipsub::Event::Subscribed { peer_id, topic: _ })) => {
                    println!("✅ Neighbor {} joined! Ready for bidding.", peer_id);
                },
                
                SwarmEvent::Behaviour(AuctionNetworkBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                    propagation_source: _peer_id, // FIXED WARNING
                    message,
                    ..
                })) => {
                    let json_str = String::from_utf8_lossy(&message.data);
                    
                    if let Ok(parsed_msg) = serde_json::from_str::<NetworkMessage>(&json_str) {
                        match parsed_msg {
                            NetworkMessage::Commit { bidder_id, commitment } => {
                                println!("📥 Received Locked Bid from {}", bidder_id);
                                received_commitments.insert(bidder_id, commitment);
                            },
                            NetworkMessage::Reveal { bidder_id, bid, blind_hex } => {
                                println!("🔓 Peer {} is revealing their bid as: {} credits!", bidder_id, bid);
                                
                                if let Some(stored_hex) = received_commitments.get(&bidder_id) {
                                    if hex::decode(stored_hex).is_ok() { // FIXED WARNING
                                        if let Ok(blind_bytes) = hex::decode(&blind_hex) {
                                            let mut r_bytes = [0u8; 32];
                                            r_bytes.copy_from_slice(&blind_bytes);
                                            let revealed_r = Scalar::from_bytes_mod_order(r_bytes);
                                            
                                            let re_calculated_point = commit(bid, revealed_r);
                                            let re_calculated_hex = hex::encode(re_calculated_point.compress().as_bytes());
                                            
                                            if &re_calculated_hex == stored_hex {
                                                println!("    ✅ VERIFICATION PASSED: The bid is mathematically valid and wasn't tampered with!");
                                            } else {
                                                println!("    ❌ VERIFICATION FAILED: Cheating detected! Hashes do not match.");
                                            }
                                        }
                                    }
                                } else {
                                    println!("    ⚠️ Ignored: They tried to reveal a bid, but we never received their commitment!");
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