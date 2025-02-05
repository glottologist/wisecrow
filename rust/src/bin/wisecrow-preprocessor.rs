use anyhow::Error;
use clap::Parser;
use config::{Config as ConfigLoader, Environment};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{migrate::MigrateError, PgPool};
use std::str::FromStr;
use thiserror::Error;
use tokio::{
    select,
    signal::unix::{signal, SignalKind},
    task::JoinHandle,
};
use tracing::{error, info, info_span, Instrument};
use wisecrow::config::Config;
use wisecrow::files::LanguageFiles;
use wisecrow::ingesting::Ingester;
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

fn send_shutdown_message(signal_type: &str) {
    info!("Received {} | Wisecrow shutting down 👋", signal_type);
}

async fn run_until_signal(handles: Vec<JoinHandle<()>>) -> anyhow::Result<()> {
    let mut term_signal = signal(SignalKind::terminate())?;
    let mut interrupt_signal = signal(SignalKind::interrupt())?;
    let mut hangup_signal = signal(SignalKind::hangup())?;

    select! {
        _ = tokio::signal::ctrl_c() => send_shutdown_message("Ctrl+C"),
        _ = interrupt_signal.recv() => send_shutdown_message("interrupt"),
        _ = hangup_signal.recv() => send_shutdown_message("hang"),
        _ = term_signal.recv() => send_shutdown_message("terminate"),
    };

    info!("Exiting chain processes");

    for handle in handles {
        handle.abort();
    }
    Ok(())
}

/// Main asynchronous entry point
#[tokio::main]
async fn main() -> Result<(), Error> {
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
        Command::Ingest(ingest_args) => {
            info!(
                "Ingesting language files for {} to {}",
                ingest_args.native_lang, ingest_args.foreign_lang
            );
            let langs = Langs::new(ingest_args.native_lang, ingest_args.foreign_lang);
            let language_files = LanguageFiles::new(&langs)?;
            let mut handles: Vec<JoinHandle<()>> = Vec::new();

            for file in language_files.files.iter() {
                handles.push(Ingester::spawn(langs.clone(), file.clone()).await);
            }
            if let Err(e) = run_until_signal(handles)
                .instrument(info_span!(parent: None, "wisecrow.signal_handlers"))
                .await
            {
                error!("Failed to run node: {e}");
            }
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
