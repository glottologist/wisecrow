//! One advisory lock per language pair.
//!
//! The lock is transaction-scoped and held on the connection that began it.
//! A session-level lock would stick to whichever pooled connection happened
//! to take it. The transaction stays open for the guarded writes only; the
//! caller does not hold it across a download.

use std::future::Future;

use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::errors::WisecrowError;

const DOMAIN: &[u8] = b"wisecrow-pipeline-pair-v1";

/// Stable bigint key for `(native, foreign)`.
#[must_use]
pub fn pair_lock_key(native: &str, foreign: &str) -> i64 {
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(native.as_bytes());
    hasher.update([0u8]);
    hasher.update(foreign.as_bytes());
    let digest = hasher.finalize();
    let mut key = [0u8; 8];
    key.copy_from_slice(&digest[..8]);
    i64::from_be_bytes(key)
}

/// Runs `work` while this pair's advisory lock is held.
///
/// The lock transaction does not time out while `work` uses other
/// connections. `work` must not call [`with_pair_lock`] for the same pair:
/// the second connection would wait for a lock this one is holding.
///
/// # Errors
///
/// Returns an error when the lock cannot be taken or released, or when
/// `work` fails. A failure rolls the lock transaction back.
pub async fn with_pair_lock<T, F, Fut>(
    pool: &PgPool,
    native: &str,
    foreign: &str,
    work: F,
) -> Result<T, WisecrowError>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, WisecrowError>>,
{
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL idle_in_transaction_session_timeout = '0'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(pair_lock_key(native, foreign))
        .execute(&mut *tx)
        .await?;

    let result = work().await;
    if result.is_ok() {
        tx.commit().await?;
    } else {
        tx.rollback().await?;
    }
    result
}
