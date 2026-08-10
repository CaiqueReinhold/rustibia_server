use argon2::password_hash::rand_core::{OsRng, RngCore};

/// 32 bytes of OS randomness, hex-encoded to 64 characters.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
}
