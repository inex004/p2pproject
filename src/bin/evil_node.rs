use libp2p::{gossipsub, noise, swarm::{NetworkBehaviour, SwarmEvent}, tcp, yamux, SwarmBuilder};
use libp2p::identity::Keypair;
use std::time::{Duration, SystemTime, UNIX_EPOCH}; // 🔥 Added Time imports
use std::error::Error;
use tokio::time;
use futures::StreamExt; 

#[derive(NetworkBehaviour)]
pub struct EvilBehaviour {
    pub gossipsub: gossipsub::Behaviour,
}

async fn launch_hacker_bot(
    bot_name: String,
    id_keys: Keypair,
    target_addr: libp2p::Multiaddr,
    timestamp_payload: u64, // 🔥 Changed from clock_payload
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

    let mut spam_interval = time::interval(Duration::from_millis(100)); 

    loop {
        tokio::select! {
            _ = spam_interval.tick() => {
                // 🔥 Updated JSON to send 'timestamp' instead of 'lamport_clock'
                let fake_json = format!(
                    r#"{{"Commit":{{"auction_id":"ENERGY_AUCTION_001","timestamp":{},"bidder_id":"{}","commitment":"fake_hex"}}}}"#, 
                    timestamp_payload, local_peer_id
                );
                let _ = swarm.behaviour_mut().gossipsub.publish(topic.clone(), fake_json.as_bytes());
            }
            event = swarm.select_next_some() => {
                if let SwarmEvent::ConnectionEstablished { .. } = event {
                    println!("💣 [{}] CONNECTION SECURED! FIRING PAYLOADS!", bot_name);
                } else if let SwarmEvent::ConnectionClosed { .. } = event {
                    println!("💀 [{}] CONNECTION TERMINATED! Node A permanently banned us!", bot_name);
                    return; 
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("==================================================");
    println!("  👿 BOOTING COORDINATED MULTI-VECTOR ATTACK 👿");
    println!("==================================================");

    let target_addr: libp2p::Multiaddr = "/ip4/127.0.0.1/tcp/8001".parse()?;

    // --- BOT 1: THE OUTSIDER (Sybil Attack) ---
    let random_keys = Keypair::generate_ed25519();
    let target_clone_1 = target_addr.clone();
    
    // Send a normal, valid physical timestamp so it only triggers the Sybil trap
    let normal_timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    
    let bot1 = tokio::spawn(async move {
        launch_hacker_bot("SYBIL_BOT".to_string(), random_keys, target_clone_1, normal_timestamp).await;
    });

    // --- BOT 2: THE INSIDER (Time-Jacking Attack) ---
    let key_file = "meter_8002.key";
    let stolen_keys = if let Ok(bytes) = std::fs::read(&key_file) {
        Keypair::from_protobuf_encoding(&bytes).unwrap()
    } else {
        println!("❌ ERROR: Could not find {} to steal. Make sure you ran Node B at least once!", key_file);
        std::process::exit(1);
    };
    let target_clone_2 = target_addr.clone();

    // 🔥 Send a timestamp 10 seconds into the future to trigger the Bounded Acceptance Window trap!
    let time_jack_timestamp = normal_timestamp + 10_000; 

    let bot2 = tokio::spawn(async move {
        launch_hacker_bot("TIME_JACKER_BOT".to_string(), stolen_keys, target_clone_2, time_jack_timestamp).await;
    });

    let _ = tokio::join!(bot1, bot2);

    println!("==================================================");
    println!("🏁 ALL HACKER BOTS DEFEATED. SCRIPT SHUTTING DOWN.");
    println!("==================================================");
    
    Ok(())
}