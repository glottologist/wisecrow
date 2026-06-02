//! Authentication primitives shared by the web login path and the CLI admin
//! commands: Argon2id password hashing, opaque session-token generation and
//! hashing, and constant-time secret comparison.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::Engine as _;
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::errors::WisecrowError;

/// Hashes a plaintext password with Argon2id and a random salt.
///
/// # Errors
///
/// Returns [`WisecrowError::InvalidInput`] if the Argon2 hasher fails.
pub fn hash_password(plain: &str) -> Result<String, WisecrowError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| WisecrowError::InvalidInput(format!("password hashing failed: {e}")))
}

/// Verifies a plaintext password against a stored Argon2 hash.
///
/// Returns `false` for a wrong password or an unparseable hash string, so the
/// caller never needs to distinguish those cases.
#[must_use]
pub fn verify_password(plain: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .is_ok()
}

/// Generates a URL-safe session token carrying 256 bits of CSPRNG entropy.
#[must_use]
pub fn generate_session_token() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

/// Returns the SHA-256 digest of a session token. Only the digest is persisted,
/// so a leaked database does not yield usable tokens.
#[must_use]
pub fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// Compares two byte strings in constant time, returning `true` only when they
/// are equal in length and content. Used for sync-client API keys so a match
/// position cannot be inferred from timing.
#[must_use]
pub fn verify_key_ct(a: &[u8], b: &[u8]) -> bool {
    bool::from(a.ct_eq(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrip() {
        let hash = hash_password("hunter2").unwrap();
        assert!(verify_password("hunter2", &hash));
        assert!(!verify_password("wrong", &hash));
    }

    #[test]
    fn unparseable_hash_does_not_verify() {
        assert!(!verify_password("anything", "not-a-valid-phc-hash"));
    }

    #[test]
    fn tokens_are_unique_and_hash_is_stable() {
        let a = generate_session_token();
        let b = generate_session_token();
        assert_ne!(a, b);
        assert_eq!(hash_token(&a), hash_token(&a));
        assert_ne!(hash_token(&a), hash_token(&b));
    }

    #[test]
    fn constant_time_key_match() {
        assert!(verify_key_ct(b"abc", b"abc"));
        assert!(!verify_key_ct(b"abc", b"abd"));
        assert!(!verify_key_ct(b"abc", b"abcd"));
    }
}
