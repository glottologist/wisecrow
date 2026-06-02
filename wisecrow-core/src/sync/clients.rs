//! Per-client sync API keys: named, individually revocable credentials stored as
//! SHA-256 hashes. Replaces the single shared `WISECROW__SYNC_API_KEY`.

use sqlx::PgPool;

use crate::auth::{generate_session_token, hash_token, verify_key_ct};
use crate::errors::WisecrowError;

pub struct SyncClientRepository;

impl SyncClientRepository {
    /// Creates a named sync client and returns its freshly generated key. Only
    /// the key's hash is stored, so the raw key is shown exactly once.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails (e.g. a duplicate name).
    pub async fn add(pool: &PgPool, name: &str) -> Result<String, WisecrowError> {
        let key = generate_session_token();
        sqlx::query("INSERT INTO sync_clients (name, key_hash) VALUES ($1, $2)")
            .bind(name)
            .bind(hash_token(&key))
            .execute(pool)
            .await?;
        Ok(key)
    }

    /// Returns `true` if `key` matches a non-revoked client. Each candidate hash
    /// is compared in constant time.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub async fn verify(pool: &PgPool, key: &str) -> Result<bool, WisecrowError> {
        let provided = hash_token(key);
        let hashes = sqlx::query_scalar::<_, Vec<u8>>(
            "SELECT key_hash FROM sync_clients WHERE revoked_at IS NULL",
        )
        .fetch_all(pool)
        .await?;
        Ok(hashes.iter().any(|h| verify_key_ct(h, &provided)))
    }

    /// Revokes the client named `name`. Returns `true` if a live client matched.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub async fn revoke(pool: &PgPool, name: &str) -> Result<bool, WisecrowError> {
        let result = sqlx::query(
            "UPDATE sync_clients SET revoked_at = now() WHERE name = $1 AND revoked_at IS NULL",
        )
        .bind(name)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Lists clients as `(name, revoked)` for the admin CLI.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub async fn list(pool: &PgPool) -> Result<Vec<(String, bool)>, WisecrowError> {
        let rows = sqlx::query_as::<_, (String, Option<chrono::DateTime<chrono::Utc>>)>(
            "SELECT name, revoked_at FROM sync_clients ORDER BY name",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(name, revoked)| (name, revoked.is_some()))
            .collect())
    }
}
