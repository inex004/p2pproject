use libp2p::{gossipsub, noise, swarm::NetworkBehaviour, tcp, yamux, Swarm, SwarmBuilder};
use libp2p::identity::Keypair;
use libp2p::kad::{self, store::MemoryStore}; 
use libp2p::identify; 
use serde::{Serialize, Deserialize};
use std::time::Duration;
use std::error::Error;
use libp2p::gossipsub::{PeerScoreParams, PeerScoreThresholds};

#[derive(Debug, Serialize, Deserialize)]
pub enum NetworkMessage {
    AnnounceAuction { auction_id: String, seller_id: String, energy_amount: u64, reserve_price: u64 },
    Commit { auction_id: String, bidder_id: String, binding_hash: String },
    Reveal { auction_id: String, bidder_id: String, bid: u64, blind_hex: String },
    Heartbeat { auction_id: String, seller_id: String },
    DeliveryComplete { auction_id: String, seller_id: String },
    
    // 🔥 NEW: Delegated Proof of Stake Packets
    IntentToValidate { auction_id: String, validator_id: String },
    Verdict { 
        auction_id: String, validator_id: String, 
        winner_id: Option<String>, clearing_price: u64, slash_list: Vec<String> 
    },
}

#[derive(NetworkBehaviour)]
pub struct AuctionNetworkBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub kademlia: kad::Behaviour<MemoryStore>,
    pub identify: identify::Behaviour, 
}

pub fn setup_swarm(id_keys: Keypair, local_peer_id: libp2p::PeerId) -> Result<Swarm<AuctionNetworkBehaviour>, Box<dyn Error>> {
    
    // --- 1. GOSSIPSUB & SCORING ---
    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(1))
        .validation_mode(gossipsub::ValidationMode::Strict)
        .max_transmit_size(2048) 
        .mesh_n_low(3)   
        .mesh_n(4)       
        .mesh_n_high(5)  
        .build()
        .expect("Valid gossipsub config");

    let mut gossipsub_behaviour = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossipsub_config,
    ).expect("Valid gossipsub behaviour");

    let thresholds = PeerScoreThresholds {
        gossip_threshold: -10.0,
        publish_threshold: -50.0,
        graylist_threshold: -80.0,
        accept_px_threshold: 10.0,
        opportunistic_graft_threshold: 20.0,
    };

    let score_params = PeerScoreParams::default();
    let _ = gossipsub_behaviour.with_peer_score(score_params, thresholds);

    let topic = gossipsub::IdentTopic::new("energy-auction");
    gossipsub_behaviour.subscribe(&topic)?;

    // --- 2. KADEMLIA DHT ---
    let store = MemoryStore::new(local_peer_id);
    let kad_config = kad::Config::new(libp2p::StreamProtocol::new("/energy-auction/kad/1.0.0"));
    let kademlia_behaviour = kad::Behaviour::with_config(local_peer_id, store, kad_config);

    // --- 3. IDENTIFY PROTOCOL (NAT/Port Discovery) ---
    let identify_config = identify::Config::new(
        "/energy-auction/id/1.0.0".to_string(),
        id_keys.public(),
    )
    .with_push_listen_addr_updates(true); 
    let identify_behaviour = identify::Behaviour::new(identify_config);

    // --- 4. COMBINE BEHAVIOURS ---
    let behaviour = AuctionNetworkBehaviour { 
        gossipsub: gossipsub_behaviour,
        kademlia: kademlia_behaviour,
        identify: identify_behaviour,
    };

    let swarm = SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
        .with_behaviour(|_| behaviour)?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    Ok(swarm)
}