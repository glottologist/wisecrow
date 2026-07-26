pub mod acquisition;
pub mod auth;
pub mod ratelimit;
pub mod sync;
pub mod tls;

use sqlx::PgPool;
use std::fmt::Debug;
use std::sync::OnceLock;

use axum::http::StatusCode;
use dioxus::prelude::ServerFnError;
use wisecrow::config::Config;

static POOL: OnceLock<PgPool> = OnceLock::new();
static SYNC_API_KEY: OnceLock<Option<String>> = OnceLock::new();

pub fn pool() -> Result<&'static PgPool, dioxus::prelude::ServerFnError> {
    POOL.get()
        .ok_or_else(|| dioxus::prelude::ServerFnError::new("Database pool not initialized"))
}

pub(crate) fn client_error(status: StatusCode, message: &str) -> ServerFnError {
    ServerFnError::ServerError {
        message: String::from(message),
        code: status.as_u16(),
        details: None,
    }
}

pub(crate) fn internal_error(operation: &str, error: &impl Debug) -> ServerFnError {
    tracing::error!(?error, operation, "request failed");
    client_error(StatusCode::INTERNAL_SERVER_ERROR, "Request failed")
}

pub fn validate_lang(code: &str) -> Result<(), dioxus::prelude::ServerFnError> {
    if !wisecrow::lang::is_valid_code(code) {
        return Err(client_error(
            StatusCode::BAD_REQUEST,
            "Invalid language code",
        ));
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

    let sync_key = cfg.sync_api_key.map(|key| String::from(key.expose()));
    SYNC_API_KEY
        .set(sync_key)
        .map_err(|_| "Sync API key already initialized")?;

    Ok(())
}

/// Builds the fullstack axum router with the auth-enrichment middleware layered
/// on. Used instead of the default `launch` so the middleware applies to every
/// request; from P4 the TLS bootstrap binds this same router.
pub fn build_router() -> axum::Router {
    dioxus::server::router(crate::app)
        .merge(sync::sync_routes())
        .layer(axum::middleware::from_fn(auth::auth_enrich_layer))
}
