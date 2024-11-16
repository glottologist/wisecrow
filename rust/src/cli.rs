use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[clap(author, version, about = "Wisecrow", long_about = "Wisecrow language")]
pub struct Cli {
    /// Subcommand to execute
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Args)]
pub struct DownloadArgs {
    #[arg(short, long)]
    pub native_lang: String,
    #[arg(short, long)]
    pub foreign_lang: String,
}

#[derive(Subcommand)]
pub enum Command {
    #[command(aliases = ["d"])]
    Download(DownloadArgs),
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_download_command_with_alias() {
        let args = Cli::parse_from(&[
            "wisecrow",
            "d",
            "--native-lang",
            "en",
            "--foreign-lang",
            "fr",
        ]);

        match args.command {
            Command::Download(download_args) => {
                assert_eq!(download_args.native_lang, "en");
                assert_eq!(download_args.foreign_lang, "fr");
            }
        }
    }
}
