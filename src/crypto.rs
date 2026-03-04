use curve25519_dalek::ristretto::RistrettoPoint;
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::constants::RISTRETTO_BASEPOINT_TABLE;
use sha2::{Sha512, Digest};

pub fn get_h_basepoint() -> RistrettoPoint {
    let mut hasher = Sha512::new();
    hasher.update(b"energy_auction_basepoint_h");
    let result = hasher.finalize(); 
    let bytes: [u8; 64] = result.into();
    RistrettoPoint::from_uniform_bytes(&bytes)
}

pub fn commit(bid_value: u64, blinding_factor: Scalar) -> RistrettoPoint {
    let g = &RISTRETTO_BASEPOINT_TABLE; 
    let h = get_h_basepoint();
    let v = Scalar::from(bid_value);
    (*g * &v) + (blinding_factor * h)
}

// We moved the messy verification logic here to keep our main loop clean!
pub fn verify_commitment(stored_hex: &str, bid: u64, blind_hex: &str) -> bool {
    // FIXED: Changed to .is_ok() to eliminate the unused variable warning
    if hex::decode(stored_hex).is_ok() {
        if let Ok(blind_bytes) = hex::decode(blind_hex) {
            let mut r_bytes = [0u8; 32];
            r_bytes.copy_from_slice(&blind_bytes);
            let revealed_r = Scalar::from_bytes_mod_order(r_bytes);
            
            let re_calculated_point = commit(bid, revealed_r);
            let re_calculated_hex = hex::encode(re_calculated_point.compress().as_bytes());
            
            return re_calculated_hex == stored_hex;
        }
    }
    false
}