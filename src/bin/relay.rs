use std::error::Error;
use std::fs;
use std::time::Duration; // <-- Added this import
use libp2p::{
    identity, noise, tcp, yamux, identify, relay, ping,
    swarm::{NetworkBehaviour, SwarmEvent},
    Multiaddr, SwarmBuilder,
};
use futures::StreamExt;

const RELAY_KEY_FILE: &str = "relay_node.key";

#[derive(NetworkBehaviour)]
struct RelayBehaviour {
    relay: relay::Behaviour,
    identify: identify::Behaviour,
    ping: ping::Behaviour,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("🚀 Starting Dedicated Cloud Relay Node...");

    let local_key = if let Ok(bytes) = fs::read(RELAY_KEY_FILE) {
        println!("🔑 Loaded existing relay keypair from disk.");
        identity::Keypair::from_protobuf_encoding(&bytes)?
    } else {
        println!("🔑 No keypair found. Generating new one and saving to '{}'.", RELAY_KEY_FILE);
        let new_key = identity::Keypair::generate_ed25519();
        fs::write(RELAY_KEY_FILE, new_key.to_protobuf_encoding()?)?;
        new_key
    };

    let local_peer_id = local_key.public().to_peer_id();
    println!("🌍 Relay Peer ID: {}", local_peer_id);

    let mut swarm = SwarmBuilder::with_existing_identity(local_key)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )?
        .with_behaviour(|key| {
            
            //// 🔥 THE FIX: Override the restrictive bandwidth and time limits!
            let relay_config = relay::Config {
                reservation_duration: Duration::from_secs(86400), // <-- Changed to match your libp2p version
                max_circuit_duration: Duration::from_secs(86400),     
                max_circuit_bytes: u64::MAX,                          
                ..Default::default()
            };

            RelayBehaviour {
                relay: relay::Behaviour::new(local_peer_id, relay_config),
                identify: identify::Behaviour::new(identify::Config::new(
                    "/energy-auction/1.0.0".into(),
                    key.public(),
                )),
                ping: ping::Behaviour::new(ping::Config::new()),
            }
        })?
        // Prevent the server from dropping idle connections
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(86400)))
        .build();

    let listen_addr: Multiaddr = "/ip4/0.0.0.0/tcp/10000".parse()?;
    swarm.listen_on(listen_addr)?;

    let external_addr: Multiaddr = "/ip4/35.216.252.107/tcp/10000".parse()?;
    swarm.add_external_address(external_addr);
    println!("🌐 Hardcoded Public External Address to: 35.216.252.107");

    println!("⏳ Waiting for connections...");

    loop {
        tokio::select! {
            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("🟢 Relay Listening on: {:?}", address);
                }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    println!("🤝 Peer connected: {}", peer_id);
                }
                SwarmEvent::Behaviour(RelayBehaviourEvent::Relay(e)) => {
                    println!("🔄 Relay Event: {:?}", e);
                }
                _ => {}
            }
        }
    }
}