use std::collections::HashMap;
use curve25519_dalek::scalar::Scalar;
use std::cmp;

const RESERVE_PRICE: u64 = 100; 

pub struct AuctionState {
    pub lamport_clock: u64, // NEW: The Lamport Logical Clock
    pub received_commitments: HashMap<String, String>,
    pub verified_bids: HashMap<String, u64>,
    pub my_secret_bid: Option<u64>,
    pub my_secret_blind: Option<Scalar>,
}

impl AuctionState {
    pub fn new() -> Self {
        Self {
            lamport_clock: 0, 
            received_commitments: HashMap::new(),
            verified_bids: HashMap::new(),
            my_secret_bid: None,
            my_secret_blind: None,
        }
    }

    // Call this right before sending a message
    pub fn tick(&mut self) -> u64 {
        self.lamport_clock += 1;
        self.lamport_clock
    }

    // Call this the exact moment a message arrives from the network
    pub fn sync_clock(&mut self, received_clock: u64) {
        self.lamport_clock = cmp::max(self.lamport_clock, received_clock) + 1;
    }

    pub fn resolve(&mut self) {
        if self.verified_bids.is_empty() {
            println!("⚠️ No verified bids available to resolve the auction.");
            return;
        }

        let mut sorted_bids: Vec<(&String, &u64)> = self.verified_bids.iter().collect();
        sorted_bids.sort_by(|a, b| b.1.cmp(a.1));

        let winner_id = sorted_bids[0].0;
        let winning_bid = *sorted_bids[0].1;

        if winning_bid < RESERVE_PRICE {
            println!("\n❌ AUCTION FAILED ❌");
            println!("The highest bid ({} credits) did not meet the Reserve Price of {}.", winning_bid, RESERVE_PRICE);
        } else {
            let mut clearing_price = if sorted_bids.len() > 1 { 
                *sorted_bids[1].1 
            } else { 
                winning_bid 
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
            }
            println!("==================================================\n");
        }

        self.received_commitments.clear();
        self.verified_bids.clear();
        self.my_secret_bid = None;
        self.my_secret_blind = None;
        
        // We tick the clock one more time to mark the end of the auction event
        self.tick(); 
        println!("🧹 Memory wiped. Lamport Clock currently at: {}\n", self.lamport_clock);
    }
}