use argon2::password_hash::rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

/// 32 bytes of OS randomness, hex-encoded to 64 characters.
///
/// This value is returned to the client **once** and never stored; what goes in the
/// database is `hash_token` of it.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_64_hex_characters() {
        let token = generate_token();
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn tokens_do_not_repeat() {
        let a = generate_token();
        let b = generate_token();
        assert_ne!(a, b);
    }

    /// The shared constant. `crates/server` asserts this exact pair, so if either side
    /// changes algorithm or encoding, that side's tests fail here rather than the two
    /// silently disagreeing at login time.
    #[test]
    fn hash_token_is_hex_sha256() {
        assert_eq!(
            hash_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_hash_is_64_hex_characters_and_deterministic() {
        let token = generate_token();
        let hash = hash_token(&token);

        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(hash, hash_token(&token), "hashing must be deterministic");
    }

    /// The point of the whole change: what lands in the database must not be the
    /// credential the client presents.
    #[test]
    fn the_stored_hash_is_not_the_token() {
        let token = generate_token();
        assert_ne!(hash_token(&token), token);
    }

    #[test]
    fn different_tokens_hash_differently() {
        assert_ne!(hash_token(&generate_token()), hash_token(&generate_token()));
    }
}
