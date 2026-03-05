pub mod learn;
#[cfg(any(feature = "audio", feature = "images"))]
pub mod media;
pub mod quiz;

use sqlx::PgPool;
use std::sync::OnceLock;

use wisecrow::config::Config;

static POOL: OnceLock<PgPool> = OnceLock::new();

/// Returns a reference to the shared database pool, initializing on first call.
///
/// # Errors
///
/// Returns an error if the pool has not been initialized.
pub fn pool() -> Result<&'static PgPool, dioxus::prelude::ServerFnError> {
    POOL.get()
        .ok_or_else(|| dioxus::prelude::ServerFnError::new("Database pool not initialized"))
}

/// Initializes the database pool from environment configuration.
///
/// # Errors
///
/// Returns an error if config loading, DB connection, or migration fails.
pub async fn init_pool() -> Result<(), Box<dyn std::error::Error>> {
    use config::Environment;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use std::str::FromStr;

    if let Err(e) = dotenv::dotenv() {
        tracing::debug!("No .env file loaded: {e}");
    }

    let settings = config::Config::builder()
        .add_source(Environment::with_prefix("WISECROW").separator("__"))
        .build()?;
    let cfg: Config = settings.try_deserialize()?;
    let database_url = cfg.database_url().map_err(|e| e.to_string())?;

    let connect_options = PgConnectOptions::from_str(&database_url)?;
    let db_pool = PgPoolOptions::new()
        .max_connections(10)
        .connect_with(connect_options)
        .await?;

    tracing::info!("Connected to database");

    sqlx::migrate!("../wisecrow-core/migrations")
        .run(&db_pool)
        .await?;

    tracing::info!("Database migrations applied");

    POOL.set(db_pool).map_err(|_| "Pool already initialized")?;

    Ok(())
}
