//! PKCE S256 helpers.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use sha2::{Digest, Sha256};

/// Generate a high-entropy code_verifier (43–128 chars, unreserved).
pub fn generate_verifier() -> String {
    let mut buf = [0u8; 32];
    let u = uuid::Uuid::new_v4();
    let v = uuid::Uuid::new_v4();
    buf[..16].copy_from_slice(u.as_bytes());
    buf[16..].copy_from_slice(v.as_bytes());
    URL_SAFE_NO_PAD.encode(buf)
}

pub fn challenge_s256(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_is_stable() {
        let v = "a".repeat(43);
        let a = challenge_s256(&v);
        let b = challenge_s256(&v);
        assert_eq!(a, b);
        assert!(a.len() >= 43);
    }
}
