// --- NETWORKING MODULE ---
// This file handles the entire libp2p stack. It creates a serverless, 
// encrypted, peer-to-peer mesh network so nodes can discover each other 
// and safely broadcast auction data.

use libp2p::{gossipsub, mdns, noise, swarm::NetworkBehaviour, tcp, yamux, Swarm, SwarmBuilder};
use libp2p::identity::Keypair;
use serde::{Serialize, Deserialize};
use std::time::Duration;
use std::error::Error;

// 1. The Data Payloads
// We use Serde to easily convert these structs into JSON bytes for transmission.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum NetworkMessage {
    // Phase 1: Sending the locked cryptographic envelope
    Commit { 
        auction_id: String, // Context separation (prevents cross-auction replay attacks)
        timestamp: u64,     // TTL marker (prevents time-delayed replay attacks)
        bidder_id: String, 
        commitment: String  // The Pedersen commitment hex string
    },
    // Phase 2: Sending the secret key to unlock the envelope
    Reveal { 
        auction_id: String, 
        timestamp: u64, 
        bidder_id: String, 
        bid: u64,           // The actual unhidden bid amount
        blind_hex: String   // The secret blinding factor 'r'
    },
}

// 2. The Network Brain
// libp2p requires us to define a custom "Behaviour" that combines different network protocols.
#[derive(NetworkBehaviour)]
pub struct AuctionNetworkBehaviour {
    // Gossipsub: A scalable publish/subscribe messaging protocol (like a decentralized Kafka)
    pub gossipsub: gossipsub::Behaviour,
    // mDNS: Multicast DNS allows nodes on the same local network (WiFi) to find each other automatically
    pub mdns: mdns::tokio::Behaviour, 
}

// 3. The Swarm Builder
// This function wires up all the cryptographic and networking rules before booting the node.
pub fn setup_swarm(id_keys: Keypair, local_peer_id: libp2p::PeerId) -> Result<Swarm<AuctionNetworkBehaviour>, Box<dyn Error>> {
    
    // --- CONFIGURE GOSSIPSUB (MESSAGING & SECURITY) ---
    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(1))
        .validation_mode(gossipsub::ValidationMode::Strict)
        // DDoS Defense: We strictly drop any packet larger than 2KB to prevent a malicious 
        // node from flooding our memory with massive garbage files.
        .max_transmit_size(2048) 
        .build()
        .expect("Valid gossipsub config");

    // Impersonation Defense: We force Gossipsub to mathematically sign every single 
    // message with the sender's Ed25519 private key. 
    let mut gossipsub_behaviour = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossipsub_config,
    ).expect("Valid gossipsub behaviour");

    // Tell the router we only care about messages tagged with this specific topic
    let topic = gossipsub::IdentTopic::new("energy-auction");
    gossipsub_behaviour.subscribe(&topic)?;

    // --- CONFIGURE MDNS (DISCOVERY) ---
    let mdns_behaviour = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?;

    // Combine them into our custom struct
    let behaviour = AuctionNetworkBehaviour { gossipsub: gossipsub_behaviour, mdns: mdns_behaviour };

    // --- BUILD THE SWARM (THE ENGINE) ---
    let swarm = SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        // The Underlay Network:
        // 1. TCP: Standard reliable connection
        // 2. Noise: End-to-End Encryption (E2EE) so no one can sniff the WiFi traffic
        // 3. Yamux: Multiplexing (allows multiple parallel streams over one TCP connection)
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
        .with_behaviour(|_| behaviour)?
        // Keep idle connections alive for 60 seconds before dropping them to save resources
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    Ok(swarm)
}