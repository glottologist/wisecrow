use std::path::Path;

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    FromRow, Sqlite, SqlitePool, Transaction,
};
use uuid::Uuid;

use crate::application::MobileError;

/// Bounded SQLite pool shared by all local repositories.
pub struct SqliteStore {
    pub(crate) pool: SqlitePool,
}

#[derive(Debug, Clone, Copy, FromRow)]
pub(crate) struct StoreScope {
    pub(crate) profile_id: Uuid,
    pub(crate) user_id: i32,
}

impl SqliteStore {
    /// Opens the local database and applies all embedded migrations.
    ///
    /// # Errors
    ///
    /// Returns [`MobileError`] when the database cannot be opened or migrated.
    pub async fn open(path: &Path) -> Result<Self, MobileError> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;

        sqlx::migrate!("./migrations").run(&pool).await?;

        Ok(Self { pool })
    }

    /// Returns the shared connection pool used by local repositories.
    #[must_use]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

pub(crate) async fn active_scope(
    transaction: &mut Transaction<'_, Sqlite>,
) -> Result<StoreScope, MobileError> {
    sqlx::query_as::<_, StoreScope>(
        "SELECT p.id AS profile_id, p.active_user_id AS user_id
         FROM profiles p
         JOIN profile_users u
           ON u.profile_id = p.id AND u.user_id = p.active_user_id
         WHERE p.active = 1",
    )
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(MobileError::Authentication)
}

pub(crate) async fn active_scope_from_pool(pool: &SqlitePool) -> Result<StoreScope, MobileError> {
    sqlx::query_as::<_, StoreScope>(
        "SELECT p.id AS profile_id, p.active_user_id AS user_id
         FROM profiles p
         JOIN profile_users u
           ON u.profile_id = p.id AND u.user_id = p.active_user_id
         WHERE p.active = 1",
    )
    .fetch_optional(pool)
    .await?
    .ok_or(MobileError::Authentication)
}

pub(crate) fn interleave_ranked<T>(words: Vec<T>, phrases: Vec<T>, size: usize) -> Vec<T> {
    let mut words = words.into_iter();
    let mut phrases = phrases.into_iter();
    let mut deck = Vec::with_capacity(size);
    while deck.len() < size {
        let next = if deck.len() % 5 == 4 {
            phrases.next().or_else(|| words.next())
        } else {
            words.next().or_else(|| phrases.next())
        };
        match next {
            Some(item) => deck.push(item),
            None => break,
        }
    }
    deck
}
