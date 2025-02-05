use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[clap(author, version, about = "Wisecrow", long_about = "Wisecrow language")]
pub struct Cli {
    /// Subcommand to execute
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Args)]
pub struct IngestArgs {
    #[arg(short, long)]
    pub native_lang: String,
    #[arg(short, long)]
    pub foreign_lang: String,
}

#[derive(Subcommand)]
pub enum Command {
    #[command(aliases = ["i"])]
    Ingest(IngestArgs),
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_download_command_with_alias() {
        let args = Cli::parse_from(&[
            "wisecrow",
            "i",
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
