pub mod acquisition;
pub mod learn;
#[cfg(any(feature = "audio", feature = "images"))]
pub mod media;
pub mod nback;
pub mod quiz;
pub mod sync;

use sqlx::PgPool;
use std::sync::OnceLock;

use wisecrow::config::Config;

static POOL: OnceLock<PgPool> = OnceLock::new();
static SYNC_API_KEY: OnceLock<Option<String>> = OnceLock::new();

const MAX_LANG_CODE_LEN: usize = 10;

pub fn pool() -> Result<&'static PgPool, dioxus::prelude::ServerFnError> {
    POOL.get()
        .ok_or_else(|| dioxus::prelude::ServerFnError::new("Database pool not initialized"))
}

pub fn validate_sync_key(provided_key: &str) -> Result<(), dioxus::prelude::ServerFnError> {
    let expected = SYNC_API_KEY
        .get()
        .ok_or_else(|| dioxus::prelude::ServerFnError::new("Server not initialised"))?;

    match expected {
        Some(key) if key == provided_key => Ok(()),
        Some(_) => Err(dioxus::prelude::ServerFnError::new(
            "Unauthorised: invalid sync API key",
        )),
        None => Err(dioxus::prelude::ServerFnError::new(
            "Sync API key not configured on server. Set WISECROW__SYNC_API_KEY.",
        )),
    }
}

pub fn validate_lang(code: &str) -> Result<(), dioxus::prelude::ServerFnError> {
    if code.is_empty()
        || code.len() > MAX_LANG_CODE_LEN
        || !code.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(dioxus::prelude::ServerFnError::new(format!(
            "Invalid language code: {code}"
        )));
    }
    Ok(())
}

pub async fn init_pool() -> Result<(), Box<dyn std::error::Error>> {
    use config::Environment;
    use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use std::str::FromStr;

    if let Err(e) = dotenvy::dotenv() {
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

    let sync_key = cfg.sync_api_key.map(|k| k.expose().to_owned());
    SYNC_API_KEY
        .set(sync_key)
        .map_err(|_| "Sync API key already initialized")?;

    Ok(())
}
