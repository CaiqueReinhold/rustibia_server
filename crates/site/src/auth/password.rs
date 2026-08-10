use std::sync::LazyLock;

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

/// An argon2id hash of a fixed string, used to spend the same work on a login attempt
/// for an unknown email as for a known one. Without it, response timing tells an
/// attacker which emails are registered.
///
/// Computed at first use rather than hardcoded as a literal. A hand-written PHC string
/// that fails to parse would make `verify_password` return early without doing any
/// argon2 work at all, silently removing the protection this exists to provide.
/// Deriving it from `hash_password` makes it valid by construction.
static DUMMY_HASH: LazyLock<String> = LazyLock::new(|| {
    hash_password("timing equalisation placeholder")
        .expect("hashing a fixed string cannot fail")
});

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("failed to hash password: {0}")]
    Hash(String),
}

pub fn hash_password(plain: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| PasswordError::Hash(e.to_string()))
}

pub fn verify_password(plain: &str, stored_hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(stored_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .is_ok()
}

/// Runs a verification against a fixed hash and discards the result. Call this on
/// the "no such account" branch of login so both branches cost the same.
pub fn spend_dummy_verification(plain: &str) {
    let _ = verify_password(plain, &DUMMY_HASH);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_succeeds() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash));
    }

    #[test]
    fn verify_rejects_the_wrong_password() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(!verify_password("Tr0ub4dor&3", &hash));
    }

    #[test]
    fn two_hashes_of_the_same_password_differ() {
        let a = hash_password("same").unwrap();
        let b = hash_password("same").unwrap();
        assert_ne!(a, b, "each hash must use a fresh salt");
    }

    #[test]
    fn verify_rejects_a_malformed_stored_hash() {
        assert!(!verify_password("anything", "not-a-phc-string"));
    }

    #[test]
    fn dummy_verification_does_not_panic() {
        spend_dummy_verification("anything");
    }

    #[test]
    fn dummy_hash_parses_so_the_timing_guard_does_real_work() {
        assert!(
            PasswordHash::new(DUMMY_HASH.as_str()).is_ok(),
            "if the dummy hash cannot be parsed, verify_password returns early without \
             running argon2 at all — the timing protection would silently do nothing \
             while every other test still passed"
        );
    }
}
