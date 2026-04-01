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
}
