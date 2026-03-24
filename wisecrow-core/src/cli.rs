use clap::{Args, Parser, Subcommand};

pub const SUPPORTED_LANGUAGE_INFO: &[(&str, &str)] = &[
    ("af", "Afrikaans"),
    ("am", "Amharic"),
    ("ar", "Arabic"),
    ("ast", "Asturian"),
    ("az", "Azerbaijani"),
    ("ba", "Bashkir"),
    ("be", "Belarusian"),
    ("bg", "Bulgarian"),
    ("bn", "Bengali"),
    ("br", "Breton"),
    ("bs", "Bosnian"),
    ("ca", "Catalan"),
    ("ceb", "Cebuano"),
    ("cs", "Czech"),
    ("cy", "Welsh"),
    ("da", "Danish"),
    ("de", "German"),
    ("el", "Greek"),
    ("en", "English"),
    ("es", "Spanish"),
    ("et", "Estonian"),
    ("fa", "Persian"),
    ("ff", "Fulah"),
    ("fi", "Finnish"),
    ("fr", "French"),
    ("fy", "Western Frisian"),
    ("ga", "Irish"),
    ("gd", "Scottish Gaelic"),
    ("gl", "Galician"),
    ("gu", "Gujarati"),
    ("ha", "Hausa"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("hr", "Croatian"),
    ("ht", "Haitian Creole"),
    ("hu", "Hungarian"),
    ("hy", "Armenian"),
    ("id", "Indonesian"),
    ("ig", "Igbo"),
    ("ilo", "Iloko"),
    ("is", "Icelandic"),
    ("it", "Italian"),
    ("ja", "Japanese"),
    ("jv", "Javanese"),
    ("ka", "Georgian"),
    ("kk", "Kazakh"),
    ("km", "Khmer"),
    ("kn", "Kannada"),
    ("ko", "Korean"),
    ("lb", "Luxembourgish"),
    ("lg", "Ganda"),
    ("ln", "Lingala"),
    ("lo", "Lao"),
    ("lt", "Lithuanian"),
    ("lv", "Latvian"),
    ("mg", "Malagasy"),
    ("mk", "Macedonian"),
    ("ml", "Malayalam"),
    ("mn", "Mongolian"),
    ("mr", "Marathi"),
    ("ms", "Malay"),
    ("my", "Burmese"),
    ("ne", "Nepali"),
    ("nl", "Dutch"),
    ("no", "Norwegian"),
    ("ns", "Northern Sotho"),
    ("oc", "Occitan"),
    ("or", "Oriya"),
    ("pa", "Panjabi"),
    ("pl", "Polish"),
    ("ps", "Pashto"),
    ("pt", "Portuguese"),
    ("ro", "Romanian"),
    ("ru", "Russian"),
    ("sd", "Sindhi"),
    ("si", "Sinhala"),
    ("sk", "Slovak"),
    ("sl", "Slovenian"),
    ("so", "Somali"),
    ("sq", "Albanian"),
    ("sr", "Serbian"),
    ("ss", "Swati"),
    ("su", "Sundanese"),
    ("sv", "Swedish"),
    ("sw", "Swahili"),
    ("ta", "Tamil"),
    ("te", "Telugu"),
    ("tg", "Tajik"),
    ("th", "Thai"),
    ("tl", "Tagalog"),
    ("tn", "Tswana"),
    ("tr", "Turkish"),
    ("uk", "Ukrainian"),
    ("ur", "Urdu"),
    ("uz", "Uzbek"),
    ("vi", "Vietnamese"),
    ("wo", "Wolof"),
    ("xh", "Xhosa"),
    ("yi", "Yiddish"),
    ("yo", "Yoruba"),
    ("zh", "Chinese"),
    ("zu", "Zulu"),
];

#[must_use]
pub fn is_supported_language(code: &str) -> bool {
    SUPPORTED_LANGUAGE_INFO.iter().any(|(c, _)| *c == code)
}

#[derive(Parser)]
#[clap(author, version, about = "Wisecrow", long_about = "Wisecrow language")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Args)]
pub struct LanguageArgs {
    #[arg(short, long)]
    pub native_lang: String,
    #[arg(short, long)]
    pub foreign_lang: String,
    #[arg(long, value_delimiter = ' ', num_args = 1..)]
    pub corpus: Option<Vec<String>>,
    #[arg(long, default_value = "102400")]
    pub max_file_size_mb: u64,
    #[arg(long, default_value = "true")]
    pub unpack: bool,
}

#[derive(Args)]
pub struct LearnArgs {
    #[arg(short, long)]
    pub native_lang: String,
    #[arg(short, long)]
    pub foreign_lang: String,
    #[arg(long, default_value = "50")]
    pub deck_size: u32,
    #[arg(long, default_value = "3000")]
    pub speed_ms: u32,
}

#[derive(Args)]
pub struct QuizArgs {
    #[arg(short, long)]
    pub pdf_path: String,
    #[arg(long, default_value = "20")]
    pub num_questions: u32,
}

#[derive(Subcommand)]
pub enum Command {
    #[command(aliases = ["d"])]
    Download(LanguageArgs),
    #[command(aliases = ["i"])]
    Ingest(LanguageArgs),
    #[command(aliases = ["r"])]
    Learn(LearnArgs),
    #[command(aliases = ["l"])]
    ListLanguages,
    #[command(aliases = ["q"])]
    Quiz(QuizArgs),
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use rstest::rstest;

    fn is_variant(cmd: &Command, name: &str) -> bool {
        matches!(
            (cmd, name),
            (Command::Download(_), "Download")
                | (Command::Ingest(_), "Ingest")
                | (Command::Learn(_), "Learn")
                | (Command::ListLanguages, "ListLanguages")
                | (Command::Quiz(_), "Quiz")
        )
    }

    #[rstest]
    #[case(&["wisecrow", "download", "-n", "en", "-f", "es"], "Download")]
    #[case(&["wisecrow", "d", "-n", "en", "-f", "fr"], "Download")]
    #[case(&["wisecrow", "ingest", "-n", "en", "-f", "es"], "Ingest")]
    #[case(&["wisecrow", "i", "-n", "ja", "-f", "en"], "Ingest")]
    #[case(&["wisecrow", "list-languages"], "ListLanguages")]
    #[case(&["wisecrow", "l"], "ListLanguages")]
    fn command_and_alias_parses(#[case] args: &[&str], #[case] expected_variant: &str) {
        let cli = Cli::parse_from(args);
        assert!(
            is_variant(&cli.command, expected_variant),
            "Expected {expected_variant} variant"
        );
    }

    #[rstest]
    #[case("xx")]
    #[case("")]
    #[case("english")]
    #[case("EN")]
    fn invalid_language_codes_rejected(#[case] code: &str) {
        assert!(!is_supported_language(code));
    }

    #[test]
    fn corpus_filter_parses() {
        let cli = Cli::parse_from([
            "wisecrow",
            "download",
            "-n",
            "en",
            "-f",
            "es",
            "--corpus",
            "cc_matrix nllb",
        ]);
        if let Command::Download(args) = cli.command {
            let corpus = args.corpus.unwrap();
            assert_eq!(corpus, vec!["cc_matrix", "nllb"]);
        } else {
            panic!("Expected Download command");
        }
    }

    #[test]
    fn default_max_file_size() {
        let cli = Cli::parse_from(["wisecrow", "download", "-n", "en", "-f", "es"]);
        if let Command::Download(args) = cli.command {
            assert_eq!(args.max_file_size_mb, 102_400);
        } else {
            panic!("Expected Download command");
        }
    }
}
