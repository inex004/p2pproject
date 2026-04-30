use std::collections::HashMap;
use curve25519_dalek::scalar::Scalar;
use std::time::{SystemTime, UNIX_EPOCH}; 

pub const STAKE_AMOUNT: u64 = 200; // Keep the escrow stake constant

pub struct Auction {
    pub auction_id: String,
    pub seller_id: String,
    pub energy_amount: u64,
    pub reserve_price: u64, // 🔥 NEW: Dynamic Reserve Price
    pub received_commitments: HashMap<String, String>, 
    pub verified_bids: HashMap<String, u64>,
    pub verified_blinds: HashMap<String, [u8; 32]>, 
    pub commit_deadline: u64, 
    pub resolved: bool, 
    
    pub is_delivering: bool,
    pub winner_id: Option<String>,
    pub last_heartbeat: u64,
    pub slashed: bool,
    
    pub clearing_price: u64,
    pub failed: bool,
    pub energy_delivered: u64, 
}

impl Auction {
    pub fn new(auction_id: String, seller_id: String, energy_amount: u64, reserve_price: u64) -> Self {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let deadline = now + 120; 
        
        Self {
            auction_id,
            seller_id,
            energy_amount,
            reserve_price, // 🔥 NEW
            received_commitments: HashMap::new(),
            verified_bids: HashMap::new(),
            verified_blinds: HashMap::new(),
            commit_deadline: deadline,
            resolved: false,
            is_delivering: false,
            winner_id: None,
            last_heartbeat: now,
            slashed: false,
            clearing_price: 0,
            failed: false,
            energy_delivered: 0, 
        }
    }

    pub fn resolve(&mut self) {
        if self.resolved { return; }
        self.resolved = true;

        if self.verified_bids.is_empty() {
            println!("\n⚠️ No bids received for Auction {}. Market closed.", self.auction_id);
            self.failed = true;
            return;
        }

        let mut network_seed = [0u8; 32];
        for blind in self.verified_blinds.values() {
            for i in 0..32 { network_seed[i] ^= blind[i]; }
        }

        let mut sorted_bids: Vec<(String, u64, [u8; 32])> = self.verified_bids.iter().map(|(peer_id, bid)| {
            let blind = self.verified_blinds.get(peer_id).unwrap();
            let mut distance = [0u8; 32];
            for i in 0..32 { distance[i] = network_seed[i] ^ blind[i]; }
            (peer_id.clone(), *bid, distance)
        }).collect();

        sorted_bids.sort_by(|a, b| {
            match b.1.cmp(&a.1) { std::cmp::Ordering::Equal => a.2.cmp(&b.2), other => other }
        });

        let winner_id = &sorted_bids[0].0;
        let winning_bid = sorted_bids[0].1;

        // 🔥 NEW: Uses dynamic self.reserve_price instead of a constant
        if winning_bid < self.reserve_price {
            println!("\n❌ AUCTION {} FAILED (Highest bid {} was below reserve of {}). ❌", self.auction_id, winning_bid, self.reserve_price);
            self.failed = true;
        } else {
            let mut price = if sorted_bids.len() > 1 { sorted_bids[1].1 } else { self.reserve_price };
            if price < self.reserve_price { price = self.reserve_price; }
            
            self.clearing_price = price;
            self.winner_id = Some(winner_id.clone());
            self.is_delivering = true;
            self.last_heartbeat = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

            println!("\n==================================================");
            println!("           🏆 AUCTION {} RESOLVED 🏆              ", self.auction_id);
            println!("🛒 Seller: {}", self.seller_id);
            println!("🥇 Winner: {}", winner_id);
            println!("💰 Bid: {} | 📉 Vickrey Price: {} (Reserve was {})", winning_bid, price, self.reserve_price);
            println!("🔌 STATUS: Switching to PHYSICAL DELIVERY phase...");
            println!("==================================================\n");
        }
    }
}

pub struct MarketplaceState {
    pub active_auctions: HashMap<String, Auction>, 
    pub current_joined_auction: Option<String>,    
    pub my_secret_bid: Option<u64>,
    pub my_secret_blind: Option<Scalar>,
    pub unplugged_meters: std::collections::HashSet<String>, 
    
    pub my_credits: u64,
    pub my_locked_credits: u64, 
    pub my_energy: u64,
    pub my_locked_energy: u64,  
}

impl MarketplaceState {
    pub fn new() -> Self {
        Self {
            active_auctions: HashMap::new(),
            current_joined_auction: None,
            my_secret_bid: None,
            my_secret_blind: None,
            unplugged_meters: std::collections::HashSet::new(),
            my_credits: 1000,
            my_locked_credits: 0,
            my_energy: 500,
            my_locked_energy: 0,
        }
    }
}