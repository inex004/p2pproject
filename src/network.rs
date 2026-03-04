use libp2p::{gossipsub, mdns, noise, swarm::NetworkBehaviour, tcp, yamux, Swarm, SwarmBuilder};
use libp2p::identity::Keypair;
use serde::{Serialize, Deserialize};
use std::time::Duration;
use std::error::Error;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum NetworkMessage {
    Commit { auction_id: String, timestamp: u64, bidder_id: String, commitment: String },
    Reveal { auction_id: String, timestamp: u64, bidder_id: String, bid: u64, blind_hex: String },
}

#[derive(NetworkBehaviour)]
pub struct AuctionNetworkBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
}

pub fn setup_swarm(id_keys: Keypair, local_peer_id: libp2p::PeerId) -> Result<Swarm<AuctionNetworkBehaviour>, Box<dyn Error>> {
    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(1))
        .validation_mode(gossipsub::ValidationMode::Strict)
        .max_transmit_size(2048) // Spam defense
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

    let swarm = SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
        .with_behaviour(|_| behaviour)?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    Ok(swarm)
}