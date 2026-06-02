use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::errors::WisecrowError;

#[derive(Debug, Clone)]
pub struct User {
    pub id: i32,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
}

pub const DEFAULT_USER_ID: i32 = 1;

pub struct UserRepository;

impl UserRepository {
    /// Creates a new user with the given display name.
    ///
    /// # Errors
    ///
    /// Returns an error if the database insert fails.
    pub async fn create(pool: &PgPool, display_name: &str) -> Result<User, WisecrowError> {
        let row = sqlx::query_as::<_, (i32, String, DateTime<Utc>)>(
            "INSERT INTO users (display_name) VALUES ($1) RETURNING id, display_name, created_at",
        )
        .bind(display_name)
        .fetch_one(pool)
        .await?;

        Ok(User {
            id: row.0,
            display_name: row.1,
            created_at: row.2,
        })
    }

    /// Fetches a user by ID, returning `None` if not found.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn get_by_id(pool: &PgPool, id: i32) -> Result<Option<User>, WisecrowError> {
        let row = sqlx::query_as::<_, (i32, String, DateTime<Utc>)>(
            "SELECT id, display_name, created_at FROM users WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(row.map(|(id, display_name, created_at)| User {
            id,
            display_name,
            created_at,
        }))
    }

    /// Lists all users ordered by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn list_all(pool: &PgPool) -> Result<Vec<User>, WisecrowError> {
        let rows = sqlx::query_as::<_, (i32, String, DateTime<Utc>)>(
            "SELECT id, display_name, created_at FROM users ORDER BY id",
        )
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, display_name, created_at)| User {
                id,
                display_name,
                created_at,
            })
            .collect())
    }

    /// Creates a user with web-login credentials (argon2 hash) and an admin flag.
    ///
    /// # Errors
    ///
    /// Returns an error if the insert fails (e.g. a duplicate email).
    pub async fn create_with_credentials(
        pool: &PgPool,
        email: &str,
        display_name: &str,
        password_hash: &str,
        is_admin: bool,
    ) -> Result<User, WisecrowError> {
        let row = sqlx::query_as::<_, (i32, String, DateTime<Utc>)>(
            "INSERT INTO users (display_name, email, password_hash, is_admin)
             VALUES ($1, $2, $3, $4) RETURNING id, display_name, created_at",
        )
        .bind(display_name)
        .bind(email)
        .bind(password_hash)
        .bind(is_admin)
        .fetch_one(pool)
        .await?;

        Ok(User {
            id: row.0,
            display_name: row.1,
            created_at: row.2,
        })
    }

    /// Sets a new password hash for the user with `email`. Returns `true` if a
    /// user matched.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub async fn set_password(
        pool: &PgPool,
        email: &str,
        password_hash: &str,
    ) -> Result<bool, WisecrowError> {
        let result = sqlx::query("UPDATE users SET password_hash = $1 WHERE email = $2")
            .bind(password_hash)
            .bind(email)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Disables web login for `email`: clears the password hash and revokes the
    /// user's active sessions. Returns `true` if a user matched.
    ///
    /// # Errors
    ///
    /// Returns an error if the update fails.
    pub async fn disable(pool: &PgPool, email: &str) -> Result<bool, WisecrowError> {
        let result = sqlx::query("UPDATE users SET password_hash = NULL WHERE email = $1")
            .bind(email)
            .execute(pool)
            .await?;
        sqlx::query(
            "DELETE FROM auth_sessions WHERE user_id IN (SELECT id FROM users WHERE email = $1)",
        )
        .bind(email)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Lists accounts with the login-relevant fields for the admin CLI:
    /// `(id, email, display_name, is_admin)`.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub async fn list_accounts(
        pool: &PgPool,
    ) -> Result<Vec<(i32, Option<String>, String, bool)>, WisecrowError> {
        let rows = sqlx::query_as::<_, (i32, Option<String>, String, bool)>(
            "SELECT id, email, display_name, is_admin FROM users ORDER BY id",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }
}
