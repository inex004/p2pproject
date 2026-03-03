use curve25519_dalek::ristretto::{RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::constants::RISTRETTO_BASEPOINT_TABLE;
use rand::thread_rng;
use sha2::{Sha512, Digest};

/// Creates a stable basepoint H by hashing a label.
/// In v4.1, we hash to 64 bytes and use from_uniform_bytes.
fn get_h_basepoint() -> RistrettoPoint {
    let mut hasher = Sha512::new();
    hasher.update(b"energy_auction_basepoint_h");
    let result = hasher.finalize(); // This gives us 64 bytes
    let bytes: [u8; 64] = result.into();
    RistrettoPoint::from_uniform_bytes(&bytes)
}

/// Creates a Pedersen Commitment: C = vG + rH
fn commit(bid_value: u64, blinding_factor: Scalar) -> RistrettoPoint {
    let g = &RISTRETTO_BASEPOINT_TABLE; // This is a table of points
    let h = get_h_basepoint();
    
    let v = Scalar::from(bid_value);
    
    // Logic: (v * G) + (r * H)
    // In v4.1, multiplying the Table 'g' requires dereferencing it: *g * &v
    let v_g = *g * &v;
    let r_h = blinding_factor * h;
    
    v_g + r_h
}

fn verify(commitment: RistrettoPoint, revealed_bid: u64, revealed_blind: Scalar) -> bool {
    let expected = commit(revealed_bid, revealed_blind);
    expected == commitment
}

fn main() {
    let mut rng = thread_rng();

    println!("--- Phase 1: Pedersen Commitment Logic ---");
    
    // 1. Participant prepares a secret bid
    let my_actual_bid = 75;
    let r = Scalar::random(&mut rng);
    
    // 2. Create the hidden commitment
    let my_commitment = commit(my_actual_bid, r);
    
    // We use the 'hex' crate we just added to display the commitment nicely
    let commitment_hex = hex::encode(my_commitment.compress().as_bytes());
    
    println!("Commitment (Publicly Shared): {}", commitment_hex);
    println!("The value '{}' is now locked and hidden.\n", my_actual_bid);

    // 3. Verification Phase (Simulation)
    println!("--- Phase 2: Verification ---");
    let is_valid = verify(my_commitment, my_actual_bid, r);

    if is_valid {
        println!("✅ Success: The revealed bid matches the commitment!");
    } else {
        println!("❌ Error: Verification failed!");
    }
}