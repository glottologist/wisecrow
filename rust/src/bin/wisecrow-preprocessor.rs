use clap::Parser;
use config::{Config as ConfigLoader, Environment};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{migrate::MigrateError, PgPool};
use std::str::FromStr;
use thiserror::Error;
use tracing::info;
use wisecrow::config::Config;
use wisecrow::{
    cli::{Cli, Command},
    downloader::Downloader,
    errors::WisecrowError,
    Langs,
};

async fn assure_db(database_url: String) -> Result<PgPool, WisecrowError> {
    let connect_options = PgConnectOptions::from_str(&database_url)?;
    let pool_options = PgPoolOptions::new().max_connections(1);
    let pool = pool_options
        .connect_with(connect_options)
        .await
        .map_err(WisecrowError::PersistenceConnectionError)?;
    info!("Connected to translations database {}", &database_url);
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(WisecrowError::PersistenceMigrationError)?;
    info!("Database migrations applied");
    Ok(pool)
}

/// Main asynchronous entry point
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing subscriber for logging
    tracing_subscriber::fmt::init();

    let settings = ConfigLoader::builder()
        .add_source(Environment::with_prefix("WISECROW").separator("__"))
        .build()?;

    let config: Config = settings
        .try_deserialize()
        .expect("Invalid wisecrow configuration");
    // Parse command-line arguments
    let cli = Cli::parse();
    dotenv::dotenv().ok();

    let url = format!(
        "postgres://{}:{}@{}/{}",
        config.db_user, config.db_password, config.db_address, config.db_name
    );
    let _ = assure_db(url).await;

    // Match on the command provided via CLI
    match cli.command {
        Command::Download(download_args) => {
            info!(
                "Downloading language files for {} to {}",
                download_args.native_lang, download_args.foreign_lang
            );
            let langs = Langs::new(download_args.native_lang, download_args.foreign_lang);
            let downloader = Downloader::new(langs).expect("Unable to define languages");
            let _ = downloader
                .download()
                .await
                .expect("Unable to download language files");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        env,
        net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    };
}
