// --- AUCTION SMART CONTRACT MODULE ---
// This module handles the state of the node (its memory) and the business logic 
// for resolving the auction. It acts as a decentralized, trustless smart contract.

use std::collections::HashMap;
use curve25519_dalek::scalar::Scalar;

// 1. The Node's Local Memory (Ledger)
// In a decentralized system, every node must independently keep track of the truth.
pub struct AuctionState {
    // Phase 1 Ledger: Stores the locked cryptographic envelopes (commitments) as they arrive.
    // Key: PeerID, Value: Hex string of the Pedersen Commitment.
    pub received_commitments: HashMap<String, String>,
    
    // Phase 2 Ledger: The "Official" Ledger. A bid is ONLY added here if the 
    // network successfully verifies the elliptic curve math during the Reveal phase.
    pub verified_bids: HashMap<String, u64>,
    
    // The Node's own secrets. We keep these in memory so we can reveal them later.
    // If the node hasn't bid yet, these are None.
    pub my_secret_bid: Option<u64>,
    pub my_secret_blind: Option<Scalar>,
}

impl AuctionState {
    // Constructor to initialize an empty state when the node boots up.
    pub fn new() -> Self {
        Self {
            received_commitments: HashMap::new(),
            verified_bids: HashMap::new(),
            my_secret_bid: None,
            my_secret_blind: None,
        }
    }

    // 2. The Vickrey Settlement Engine
    // This is the core economic logic of the thesis. It determines the winner 
    // and calculates the fair market clearing price.
    pub fn resolve(&self) {
        // Safety check: Don't resolve if nobody proved a valid bid.
        if self.verified_bids.is_empty() {
            println!("⚠️ No verified bids available to resolve the auction.");
            return;
        }

        // --- MICROECONOMICS: THE SORTING HAT ---
        // Convert the HashMap into a list of tuples so we can sort it.
        // We sort by the bid amount (b.1 and a.1) in descending order (Highest to Lowest).
        let mut sorted_bids: Vec<(&String, &u64)> = self.verified_bids.iter().collect();
        sorted_bids.sort_by(|a, b| b.1.cmp(a.1));

        // 1st Place: The Highest Bidder wins the right to the energy.
        let winner_id = sorted_bids[0].0;
        let winning_bid = *sorted_bids[0].1;
        
        // --- GAME THEORY: THE SECOND-PRICE CLEARING MECHANISM ---
        // To prevent "bid shading" and force nodes to honestly bid their true maximum 
        // willingness to pay, the winner only pays the price of the SECOND highest bid.
        // If there is only one bidder in the whole network, they simply pay their own bid.
        let clearing_price = if sorted_bids.len() > 1 { 
            *sorted_bids[1].1 
        } else { 
            winning_bid 
        };

        // Print the official decentralized receipt
        println!("\n==================================================");
        println!("             🏆 AUCTION RESOLVED 🏆               ");
        println!("==================================================");
        println!("🥇 Winner Peer: {}", winner_id);
        println!("💰 Maximum Bid Willingness: {} credits", winning_bid);
        println!("📉 VICKREY CLEARING PRICE:  {} credits", clearing_price);
        
        // Highlight the market efficiency (how much the winner saved by bidding honestly)
        if winning_bid > clearing_price {
            println!("✨ The winner saved {} credits due to Vickrey rules!", winning_bid - clearing_price);
        }
        println!("==================================================\n");
    }
}