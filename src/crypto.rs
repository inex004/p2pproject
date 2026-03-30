// --- CRYPTOGRAPHY ENGINE MODULE ---
// This file handles all the Elliptic Curve Cryptography (ECC).
// It implements "Pedersen Commitments," which allow nodes to lock in a bid 
// (binding) without revealing the amount to the network (hiding).

use curve25519_dalek::ristretto::RistrettoPoint;
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::constants::RISTRETTO_BASEPOINT_TABLE;
use sha2::{Sha512, Digest};

// 1. The "Nothing-Up-My-Sleeve" Basepoint Generator
// Pedersen commitments require two independent basepoints on the elliptic curve: G and H.
// The relationship (discrete logarithm) between G and H must be completely unknown.
// We generate H by hashing a hardcoded string. This proves to the network that we 
// didn't mathematically engineer H to create a backdoor.
pub fn get_h_basepoint() -> RistrettoPoint {
    let mut hasher = Sha512::new();
    hasher.update(b"energy_auction_basepoint_h");
    let result = hasher.finalize(); 
    let bytes: [u8; 64] = result.into();
    // Maps the Sha512 hash safely onto the Ristretto255 elliptic curve
    RistrettoPoint::from_uniform_bytes(&bytes)
}

// 2. The Cryptographic Vault (Pedersen Commitment)
// The math formula is: C = vG + rH
// 'v' = the bid value. 'r' = the random secret key (blinding factor).
// This provides two absolute cryptographic guarantees:
// Hiding: Looking at C, it is mathematically impossible to figure out 'v'.
// Binding: Once C is published, you cannot find a new 'v' and 'r' that equal C.
pub fn commit(bid_value: u64, blinding_factor: Scalar) -> RistrettoPoint {
    let g = &RISTRETTO_BASEPOINT_TABLE; // The standard basepoint (G)
    let h = get_h_basepoint();          // Our custom basepoint (H)
    
    let v = Scalar::from(bid_value);
    
    // Execute the curve multiplication and addition: C = (v * G) + (r * H)
    (*g * &v) + (blinding_factor * h)
}

// 3. The Zero-Trust Verification
// During the "Reveal" phase, a node publishes their actual bid and their secret key.
// Every other node independently runs this function to verify they aren't cheating.
pub fn verify_commitment(stored_hex: &str, bid: u64, blind_hex: &str) -> bool {
    // We only proceed if the stored string is actually valid hex
    if hex::decode(stored_hex).is_ok() {
        
        // Try to decode the secret key (r) that they just revealed over the network
        if let Ok(blind_bytes) = hex::decode(blind_hex) {
            
            // Convert the raw bytes back into an Elliptic Curve Scalar
            let mut r_bytes = [0u8; 32];
            r_bytes.copy_from_slice(&blind_bytes);
            let revealed_r = Scalar::from_bytes_mod_order(r_bytes);
            
            // THE ULTIMATE TEST: 
            // We run their revealed bid and revealed key through the math formula ourselves.
            let re_calculated_point = commit(bid, revealed_r);
            let re_calculated_hex = hex::encode(re_calculated_point.compress().as_bytes());
            
            // If our calculated result matches the envelope locked in our memory from Phase 1,
            // they are telling the truth. If it doesn't match, they tried to change their bid.
            return re_calculated_hex == stored_hex;
        }
    }
    // If the hex is garbage or the math fails, reject the bid.
    false
}