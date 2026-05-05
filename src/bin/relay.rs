use std::error::Error;
use std::fs;
use std::time::Duration; 
use libp2p::{
    identity, identify, relay, ping, 
    kad, kad::store::MemoryStore, 
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
    kad: kad::Behaviour<MemoryStore>, 
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("🚀 Starting Dedicated Cloud Relay Node (QUIC/UDP)...");

    let local_key = if let Ok(bytes) = fs::read(RELAY_KEY_FILE) {
        println!("🔑 Loaded existing relay keypair from disk.");
        identity::Keypair::from_protobuf_encoding(&bytes)?
    } else {
        println!("🔑 No keypair found. Generating new one...");
        let new_key = identity::Keypair::generate_ed25519();
        fs::write(RELAY_KEY_FILE, new_key.to_protobuf_encoding()?)?;
        new_key
    };

    let local_peer_id = local_key.public().to_peer_id();
    println!("🌍 Relay Peer ID: {}", local_peer_id);

    let mut swarm = SwarmBuilder::with_existing_identity(local_key)
        .with_tokio()
        .with_quic() 
        .with_behaviour(|key| {
            // 🔥 FIX: Massively expanded the capacity limits to prevent 'NoReservation' errors
            let relay_config = relay::Config {
                max_reservations: 8192,           // Default is 128. Expanded to 8000+
                max_reservations_per_peer: 100,   // Allow the same peer to reconnect during testing
                max_circuits: 1024,               // Allow more active routed connections
                max_circuits_per_peer: 100,       
                reservation_duration: Duration::from_secs(86400),
                max_circuit_duration: Duration::from_secs(86400),     
                max_circuit_bytes: u64::MAX,                          
                ..Default::default()
            };

            let store = MemoryStore::new(local_peer_id);
            let mut kademlia = kad::Behaviour::new(local_peer_id, store);
            kademlia.set_mode(Some(kad::Mode::Server)); 

            RelayBehaviour {
                relay: relay::Behaviour::new(local_peer_id, relay_config),
                identify: identify::Behaviour::new(identify::Config::new("/energy-auction/1.0.0".into(), key.public())),
                ping: ping::Behaviour::new(ping::Config::new()),
                kad: kademlia, 
            }
        })?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(86400)))
        .build();

    let listen_addr: Multiaddr = "/ip4/0.0.0.0/udp/10000/quic-v1".parse()?;
    swarm.listen_on(listen_addr)?;

    let external_addr: Multiaddr = "/ip4/35.216.252.107/udp/10000/quic-v1".parse()?;
    swarm.add_external_address(external_addr);
    println!("🌐 Hardcoded Public External Address to: 35.216.252.107 (UDP)");

    println!("⏳ Waiting for QUIC connections...");

    loop {
        tokio::select! {
            event = swarm.select_next_some() => match event {
                SwarmEvent::NewListenAddr { address, .. } => println!("🟢 Relay Listening on: {:?}", address),
                SwarmEvent::ConnectionEstablished { peer_id, endpoint, .. } => {
                    println!("🤝 Peer connected via QUIC: {}", peer_id);
                    swarm.behaviour_mut().kad.add_address(&peer_id, endpoint.get_remote_address().clone());
                },
                // 🔥 FIX: Added logging so you can physically see the Relay accepting routing slots
                SwarmEvent::Behaviour(RelayBehaviourEvent::Relay(relay_event)) => {
                    match relay_event {
                        relay::Event::ReservationReqAccepted { src_peer_id, .. } => {
                            println!("🎟️ [RELAY]: Granted Reservation routing slot to peer {}", &src_peer_id.to_string()[0..8]);
                        },
                        relay::Event::CircuitReqAccepted { src_peer_id, dst_peer_id } => {
                            println!("🔗 [RELAY]: Bridging a p2p-circuit between {} and {}", &src_peer_id.to_string()[0..8], &dst_peer_id.to_string()[0..8]);
                        },
                        relay::Event::ReservationReqDenied { src_peer_id } => {
                            println!("❌ [RELAY]: Denied reservation for {}", &src_peer_id.to_string()[0..8]);
                        },
                        _ => {}
                    }
                },
                _ => {}
            }
        }
    }
}