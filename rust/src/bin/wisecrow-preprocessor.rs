use clap::Parser;
use tracing::info;
use wisecrow::cli::{Cli, Command};

/// Main asynchronous entry point
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing subscriber for logging
    tracing_subscriber::fmt::init();

    // Parse command-line arguments
    let cli = Cli::parse();
    dotenv::dotenv().ok();

    // Match on the command provided via CLI
    match cli.command {
        Command::Download(download_args) => {
            info!(
                "Downloading language files for {} to {}",
                download_args.native_lang, download_args.foreign_lang
            );
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