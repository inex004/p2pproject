use std::collections::HashMap;
use curve25519_dalek::scalar::Scalar;

pub struct AuctionState {
    pub received_commitments: HashMap<String, String>,
    pub verified_bids: HashMap<String, u64>,
    pub my_secret_bid: Option<u64>,
    pub my_secret_blind: Option<Scalar>,
}

impl AuctionState {
    pub fn new() -> Self {
        Self {
            received_commitments: HashMap::new(),
            verified_bids: HashMap::new(),
            my_secret_bid: None,
            my_secret_blind: None,
        }
    }

    pub fn resolve(&self) {
        if self.verified_bids.is_empty() {
            println!("⚠️ No verified bids available to resolve the auction.");
            return;
        }

        let mut sorted_bids: Vec<(&String, &u64)> = self.verified_bids.iter().collect();
        sorted_bids.sort_by(|a, b| b.1.cmp(a.1));

        let winner_id = sorted_bids[0].0;
        let winning_bid = *sorted_bids[0].1;
        let clearing_price = if sorted_bids.len() > 1 { *sorted_bids[1].1 } else { winning_bid };

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
}