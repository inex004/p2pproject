use std::collections::HashMap;
use curve25519_dalek::scalar::Scalar;
use std::time::{SystemTime, UNIX_EPOCH}; 

const RESERVE_PRICE: u64 = 100; 

pub struct AuctionState {
    // 🔥 REMOVED: lamport_clock
    // The u64 here will now store the physical timestamp (ms)
    pub received_commitments: HashMap<String, (String, u64)>, 
    pub verified_bids: HashMap<String, u64>,
    pub my_secret_bid: Option<u64>,
    pub my_secret_blind: Option<Scalar>,
    pub commit_deadline: u64, 
}

impl AuctionState {
    pub fn new() -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let deadline = now + 300; // 5 minute deadline
        
        Self {
            received_commitments: HashMap::new(),
            verified_bids: HashMap::new(),
            my_secret_bid: None,
            my_secret_blind: None,
            commit_deadline: deadline, 
        }
    }

    // 🔥 REMOVED: tick() and sync_clock() functions. They are no longer needed!

    pub fn resolve(&mut self) {
        if self.verified_bids.is_empty() {
            println!("⚠️ No verified bids available to resolve the auction.");
            return;
        }

        // 🔥 FIX: We now extract the PeerID, Bid Amount, AND the Commitment Hex
        let mut sorted_bids: Vec<(&String, &u64, String)> = self.verified_bids.iter().map(|(peer_id, bid)| {
            let commit_hex = self.received_commitments.get(peer_id).unwrap().0.clone();
            (peer_id, bid, commit_hex)
        }).collect();

        // 🔥 FIX: The Cryptographic Lottery Tie-Breaker
        sorted_bids.sort_by(|a, b| {
            match b.1.cmp(a.1) { // Primary Sort: Bid Amount (Highest wins)
                std::cmp::Ordering::Equal => {
                    // Secondary Sort (Tie-Breaker): Compare the random commitment strings
                    // This is perfectly fair, unpredictable to the user, and 100% deterministic across the network!
                    b.2.cmp(&a.2) 
                },
                other => other,
            }
        });

        let winner_id = sorted_bids[0].0;
        let winning_bid = *sorted_bids[0].1;

        if winning_bid < RESERVE_PRICE {
            println!("\n❌ AUCTION FAILED ❌");
            println!("The highest bid ({} credits) did not meet the Reserve Price of {}.", winning_bid, RESERVE_PRICE);
        } else {
            // Note on Vickrey Ties: If Alice and Bob both bid 150, the winner pays the second-highest bid.
            // In a tie, the second-highest bid is ALSO 150. So the winner pays 150. This is economically correct!
            let mut clearing_price = if sorted_bids.len() > 1 { 
                *sorted_bids[1].1 
            } else { 
                RESERVE_PRICE 
            };

            if clearing_price < RESERVE_PRICE {
                println!("⚠️ WARNING: Anti-Collusion triggered. Adjusting clearing price to Reserve Minimum.");
                clearing_price = RESERVE_PRICE;
            }

            println!("\n==================================================");
            println!("             🏆 AUCTION RESOLVED 🏆               ");
            println!("==================================================");
            println!("🥇 Winner Peer: {}", winner_id);
            println!("💰 Maximum Bid Willingness: {} credits", winning_bid);
            println!("📉 VICKREY CLEARING PRICE:  {} credits", clearing_price);
            
            if winning_bid > clearing_price {
                println!("✨ The winner saved {} credits due to Vickrey rules!", winning_bid - clearing_price);
            } else {
                println!("⚖️ Flat clearing price applied (Tie or Reserve).");
            }
            println!("==================================================\n");
        }

        self.received_commitments.clear();
        self.verified_bids.clear();
        self.my_secret_bid = None;
        self.my_secret_blind = None;
        
        println!("🧹 Memory wiped. State machine reset.\n");
    }
}