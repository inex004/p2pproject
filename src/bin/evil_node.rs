use libp2p::{gossipsub, noise, swarm::{NetworkBehaviour, SwarmEvent}, tcp, yamux, SwarmBuilder};
use libp2p::identity::Keypair;
use std::time::Duration;
use std::error::Error;
use tokio::time;
use futures::StreamExt;


#[derive(NetworkBehaviour)]
pub struct EvilBehaviour {
    pub gossipsub: gossipsub::Behaviour,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("😈 BOOTING EVIL NODE (DDoS SPAMMER) 😈");

    let id_keys = Keypair::generate_ed25519();
    let local_peer_id = id_keys.public().to_peer_id();
    println!("My Hacker Peer ID: {}", local_peer_id);

    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .validation_mode(gossipsub::ValidationMode::Strict)
        .build()
        .expect("Valid config");

    let mut gossipsub_behaviour = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossipsub_config,
    ).expect("Valid behaviour");

    let topic = gossipsub::IdentTopic::new("energy-auction");
    gossipsub_behaviour.subscribe(&topic)?;

    let behaviour = EvilBehaviour { gossipsub: gossipsub_behaviour };

    let mut swarm = SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default)?
        .with_behaviour(|_| behaviour)?
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    // Target Node A
    let target_addr: libp2p::Multiaddr = "/ip4/127.0.0.1/tcp/8001".parse()?;
    println!("🎯 Targeting Node A at {}...", target_addr);
    swarm.dial(target_addr)?;

    let mut spam_interval = time::interval(Duration::from_millis(50)); // 20 messages a second!

    loop {
        tokio::select! {
            _ = spam_interval.tick() => {
                // Create a JSON that is structurally valid, but from an unauthorized peer
                let fake_json = format!(r#"{{"Commit":{{"auction_id":"ENERGY_AUCTION_001","lamport_clock":999,"bidder_id":"{}","commitment":"fake_hex"}}}}"#, local_peer_id);
                let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), fake_json.as_bytes());
            }
            event = swarm.select_next_some() => {
                if let SwarmEvent::ConnectionEstablished { .. } = event {
                    println!("💣 CONNECTION SECURED! COMMENCING SPAM ATTACK!");
                } else if let SwarmEvent::ConnectionClosed { .. } = event {
                    println!("❌ CONNECTION LOST! Node A banned our IP!");
                    std::process::exit(0);
                }
            }
        }
    }
}