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
    /// Ceiling on the decompressed size of a single archive, in MB. Raise it
    /// for the larger NLLB releases, which expand to several gigabytes.
    #[arg(long, default_value = "8192")]
    pub max_decompressed_mb: u64,
    #[arg(long, default_value = "true")]
    pub unpack: bool,
}

#[derive(Args)]
pub struct IngestArgs {
    #[command(flatten)]
    pub langs: LanguageArgs,
    /// Ingest an already-downloaded TMX file instead of fetching from OPUS.
    /// The corpus and download options are ignored when this is given.
    #[arg(long)]
    pub file: Option<std::path::PathBuf>,
    /// Exact `xml:lang` tag the file uses for the native language, when it
    /// differs from the application code (for example `zh_CN` for `zh`).
    #[arg(long, requires = "file")]
    pub tmx_source_lang: Option<String>,
    /// Exact `xml:lang` tag the file uses for the foreign language, when it
    /// differs from the application code (for example `zh_CN` for `zh`).
    #[arg(long, requires = "file")]
    pub tmx_target_lang: Option<String>,
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
    #[arg(long, default_value = "1")]
    pub user_id: i32,
}

#[derive(Args)]
pub struct QuizArgs {
    #[arg(short, long)]
    pub pdf_path: String,
    #[arg(long, default_value = "20")]
    pub num_questions: u32,
}

#[derive(Args)]
pub struct DownloadAllArgs {
    #[arg(short, long)]
    pub native_lang: String,
    #[arg(short, long)]
    pub output_dir: String,
    #[arg(long, value_delimiter = ' ', num_args = 1..)]
    pub corpus: Option<Vec<String>>,
    #[arg(long, default_value = "102400")]
    pub max_file_size_mb: u64,
    /// Ceiling on the decompressed size of a single archive, in MB. Raise it
    /// for the larger NLLB releases, which expand to several gigabytes.
    #[arg(long, default_value = "8192")]
    pub max_decompressed_mb: u64,
    #[arg(long, default_value = "true")]
    pub unpack: bool,
}

#[derive(Args)]
pub struct SeedGrammarArgs {
    #[arg(short, long)]
    pub lang: String,
    #[arg(short = 'L', long, value_delimiter = ',')]
    pub levels: Vec<String>,
}

#[derive(Args)]
pub struct ImportGrammarArgs {
    #[arg(short, long)]
    pub lang: String,
    #[arg(short, long)]
    pub file: String,
}

#[derive(Args)]
pub struct ImportPdfArgs {
    #[arg(short, long)]
    pub lang: String,
    #[arg(short = 'L', long)]
    pub level: String,
    #[arg(short, long)]
    pub file: String,
}

#[derive(Args)]
pub struct SyncArgs {
    #[arg(short, long)]
    pub remote: String,
    #[arg(long)]
    pub api_key: Option<String>,
}

#[derive(Args)]
pub struct GenerateExercisesArgs {
    #[arg(short, long)]
    pub lang: String,
    #[arg(short = 'L', long)]
    pub level: String,
    #[arg(short, long, default_value = "20")]
    pub count: u32,
}

#[derive(Args)]
pub struct FrequencyArgs {
    #[arg(short, long)]
    pub lang: String,
    /// Update from a local frequency file instead of downloading the Hermit
    /// Dave list. Reads `word count`, `rank<TAB>word<TAB>count` (Leipzig) and
    /// `word,count` (CSV) layouts.
    #[arg(long, conflicts_with = "from_corpus")]
    pub file: Option<String>,
    /// Derive the frequencies from the phrases already ingested for this
    /// language instead of using a published list. The route for languages
    /// nobody has published a list for.
    #[arg(long, default_value_t = false)]
    pub from_corpus: bool,
}

#[derive(Args)]
pub struct PruneArgs {
    #[arg(short, long)]
    pub lang: String,
    /// Language of the phrase a learner reads as the prompt. Given this, the
    /// prune fetches its published word list and demotes pairs whose prompt
    /// holds no recognised word — corpus corruption such as "Bthey" or
    /// "andthatthe", which no ordering rule inside the deck query can catch.
    #[arg(short, long)]
    pub native_lang: Option<String>,
    /// Report what would change without writing anything. Worth running first:
    /// a corpus that turns out to be mostly the wrong language loses most of
    /// its rows, and that is better seen than discovered.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct GlossDeckArgs {
    /// The language being learned.
    #[arg(short, long)]
    pub lang: String,
    /// The language the learner reads the prompt in.
    #[arg(short, long)]
    pub native_lang: String,
    /// Pending rows to scan in this window (1–1000). Rows that are
    /// punctuation, digits or the wrong script are reported and skipped, so
    /// fewer than this may be enriched. Repeated runs skip words at the
    /// current presentation version and continue down the frequency order.
    #[arg(long, default_value_t = 200)]
    pub limit: u32,
    /// Pending rows to skip before the window. Use it to step past rejected
    /// windows during one inspection pass; after enrichment has written
    /// entries, restart from 0, since accepted words leave the pending list.
    #[arg(long, default_value_t = 0)]
    pub offset: u32,
    /// List pending words without calling the model or writing anything.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[derive(Args)]
pub struct SentenceCardArgs {
    /// The language being learned.
    #[arg(short, long)]
    pub lang: String,
    /// The language the learner reads.
    #[arg(short, long)]
    pub native_lang: String,
    /// Whose known words decide what is within reach.
    #[arg(short, long)]
    pub user_id: i32,
    /// Teach this word. Omitted, the next word the deck would serve is used,
    /// which is the loop the two decks are meant to form.
    #[arg(short, long)]
    pub word: Option<String>,
}

#[derive(Args)]
pub struct ScoreSentencesArgs {
    /// The language being learned. Its words must already be ranked
    /// (`frequency --from-corpus`), since a sentence is scored by the words in
    /// it.
    #[arg(short, long)]
    pub lang: String,
}

#[derive(Args)]
pub struct NbackArgs {
    #[arg(short, long)]
    pub native_lang: String,
    #[arg(short, long)]
    pub foreign_lang: String,
    #[arg(short, long, default_value = "audio_written")]
    pub mode: String,
    #[arg(long, default_value = "2")]
    pub n_level: u8,
    #[arg(long, default_value = "1")]
    pub user_id: i32,
}

#[derive(Args)]
pub struct PrefetchMediaArgs {
    #[arg(short, long)]
    pub native_lang: String,
    #[arg(short, long)]
    pub foreign_lang: String,
    /// Entries of the preparation deck to prepare in this run (1–5,000).
    #[arg(long, default_value_t = 100)]
    pub limit: u32,
    /// Deck position to start from; offset plus limit may not pass 10,000.
    #[arg(long, default_value_t = 0)]
    pub offset: u32,
    /// Generated payload bytes this run may admit (default 64 MiB, at most
    /// 5 GiB). Cache hits cost nothing; a refused payload is not written.
    #[arg(long, default_value_t = 67_108_864)]
    pub max_bytes: u64,
    /// Report cached and missing media without generating anything. Opens
    /// the database and cache read-only and applies no migrations.
    #[arg(long)]
    pub dry_run: bool,
    /// Prepare speech (`--audio=false` to skip).
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    pub audio: bool,
    /// Prepare images (`--images=false` to skip).
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true")]
    pub images: bool,
}

#[derive(Args)]
pub struct ExtractPhrasesArgs {
    /// The language whose corpus is mined for frequent phrases.
    #[arg(short, long)]
    pub lang: String,
}

#[derive(Args)]
pub struct TranslatePhrasesArgs {
    /// The language the phrases belong to.
    #[arg(short, long)]
    pub lang: String,
    /// The language the learner reads the translation in.
    #[arg(short, long)]
    pub native_lang: String,
    /// Ceiling on phrases sent to the model in one run.
    #[arg(long, default_value_t = 100)]
    pub limit: u32,
    /// Re-gloss already-translated phrases, updating linked rows in place.
    #[arg(long, default_value_t = false)]
    pub refresh: bool,
}

#[derive(Args)]
pub struct GlossArgs {
    #[arg(short, long)]
    pub sentence: String,
    #[arg(short, long)]
    pub lang: String,
    /// Bypass and overwrite the cached gloss for this sentence (forces LLM re-prompt).
    #[arg(long, default_value_t = false)]
    pub refresh: bool,
}

#[derive(Args)]
pub struct ExtractWordsArgs {
    #[command(flatten)]
    pub langs: LanguageArgs,
    /// Most frequent sentence words to publish as candidates (1–10,000).
    #[arg(long, default_value_t = 500)]
    pub limit: u32,
    /// Sentences a word must occur in before it becomes a candidate (at least 2).
    #[arg(long, default_value_t = 5)]
    pub min_occurrences: u32,
}

/// Which candidates a promotion run attempts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum PromoteMode {
    /// Candidates never attempted.
    Pending,
    /// Pending candidates and those whose last attempt failed.
    RetryFailed,
    /// Every candidate, refreshing existing presentations under the same IDs.
    All,
}

#[derive(Args)]
pub struct PromoteWordsArgs {
    #[command(flatten)]
    pub langs: LanguageArgs,
    /// Candidates to attempt in this run (1–1,000).
    #[arg(long, default_value_t = 200)]
    pub limit: u32,
    #[arg(long, value_enum, default_value_t = PromoteMode::Pending)]
    pub mode: PromoteMode,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum GradedReaderFormat {
    Md,
    Html,
}

#[derive(Args)]
pub struct PreviewArgs {
    #[arg(long)]
    pub file: std::path::PathBuf,
    #[arg(short, long)]
    pub native_lang: String,
    #[arg(short, long)]
    pub foreign_lang: String,
    #[arg(long, default_value_t = false)]
    pub unknown_only: bool,
    #[arg(long, default_value_t = false)]
    pub no_srs: bool,
    #[arg(long)]
    pub top_n: Option<u32>,
    /// Use the LLM to translate tokens that aren't in the corpus (Status::Unknown).
    /// Requires `WISECROW__LLM_PROVIDER` and `WISECROW__LLM_API_KEY`.
    #[arg(long, default_value_t = false)]
    pub gloss_unknowns: bool,
    #[arg(long, default_value = "1")]
    pub user_id: i32,
}

#[derive(Args)]
pub struct GradedReaderArgs {
    #[arg(short, long)]
    pub native_lang: String,
    #[arg(short, long)]
    pub foreign_lang: String,
    #[arg(long)]
    pub cefr: String,
    #[arg(long, value_delimiter = ',', default_values_t = vec![2_i16])]
    pub seed_states: Vec<i16>,
    #[arg(long)]
    pub seed_min_stability: Option<f32>,
    #[arg(long, default_value_t = 30)]
    pub seed_limit: u32,
    #[arg(long, default_value_t = 200)]
    pub length_words: u32,
    #[arg(long, value_enum, default_value_t = GradedReaderFormat::Md)]
    pub format: GradedReaderFormat,
    #[arg(long)]
    pub output: Option<std::path::PathBuf>,
    #[arg(long, default_value = "1")]
    pub user_id: i32,
}

#[derive(Subcommand)]
pub enum UserCmd {
    /// Create a new account (prompts for a password).
    Add(UserAddArgs),
    /// List accounts.
    List,
    /// Reset an account's password (prompts for a new one).
    Passwd(UserEmailArgs),
    /// Disable web login for an account and revoke its sessions.
    Disable(UserEmailArgs),
}

#[derive(Args)]
pub struct UserAddArgs {
    #[arg(short, long)]
    pub email: String,
    #[arg(short, long)]
    pub display_name: String,
    #[arg(long, default_value_t = false)]
    pub admin: bool,
}

#[derive(Args)]
pub struct UserEmailArgs {
    #[arg(short, long)]
    pub email: String,
}

#[derive(Subcommand)]
pub enum SyncClientCmd {
    /// Create a sync client key (printed once).
    Add(SyncClientNameArgs),
    /// Revoke a sync client key.
    Revoke(SyncClientNameArgs),
    /// List sync clients.
    List,
}

#[derive(Args)]
pub struct SyncClientNameArgs {
    #[arg(short, long)]
    pub name: String,
}

#[derive(Subcommand)]
pub enum Command {
    #[command(aliases = ["d"])]
    Download(LanguageArgs),
    #[command(aliases = ["da"])]
    DownloadAll(DownloadAllArgs),
    #[command(aliases = ["ge"])]
    GenerateExercises(GenerateExercisesArgs),
    #[command(aliases = ["fr"])]
    Frequency(FrequencyArgs),
    #[command(aliases = ["gl"])]
    Gloss(GlossArgs),
    #[command(aliases = ["gr"])]
    GradedReader(GradedReaderArgs),
    #[command(aliases = ["i"])]
    Ingest(IngestArgs),
    #[command(aliases = ["ig"])]
    ImportGrammar(ImportGrammarArgs),
    #[command(aliases = ["ip"])]
    ImportPdf(ImportPdfArgs),
    #[command(aliases = ["r"])]
    Learn(LearnArgs),
    #[command(aliases = ["nb"])]
    Nback(NbackArgs),
    #[command(aliases = ["l"])]
    ListLanguages,
    #[command(aliases = ["pm"])]
    PrefetchMedia(PrefetchMediaArgs),
    /// Extract frequent multi-word phrases from the corpus into staging.
    ExtractPhrases(ExtractPhrasesArgs),
    /// Translate staged phrases with the LLM and promote them into decks.
    TranslatePhrases(TranslatePhrasesArgs),
    /// Count frequent words across corpus sentences into word candidates.
    ExtractWords(ExtractWordsArgs),
    /// Give word candidates canonical meanings and link them to learning rows.
    PromoteWords(PromoteWordsArgs),
    #[command(aliases = ["pv"])]
    Preview(PreviewArgs),
    #[command(aliases = ["gd"])]
    GlossDeck(GlossDeckArgs),
    #[command(aliases = ["pr"])]
    Prune(PruneArgs),
    #[command(aliases = ["q"])]
    Quiz(QuizArgs),
    #[command(aliases = ["sent"])]
    SentenceCard(SentenceCardArgs),
    #[command(aliases = ["ss"])]
    ScoreSentences(ScoreSentencesArgs),
    #[command(aliases = ["sg"])]
    SeedGrammar(SeedGrammarArgs),
    #[command(aliases = ["s"])]
    Sync(SyncArgs),
    #[command(aliases = ["u"])]
    User {
        #[command(subcommand)]
        command: UserCmd,
    },
    #[command(aliases = ["sc"])]
    SyncClient {
        #[command(subcommand)]
        command: SyncClientCmd,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use proptest::prelude::*;
    use rstest::rstest;

    fn is_variant(cmd: &Command, name: &str) -> bool {
        matches!(
            (cmd, name),
            (Command::Download(_), "Download")
                | (Command::DownloadAll(_), "DownloadAll")
                | (Command::GenerateExercises(_), "GenerateExercises")
                | (Command::Frequency(_), "Frequency")
                | (Command::Gloss(_), "Gloss")
                | (Command::GradedReader(_), "GradedReader")
                | (Command::ImportGrammar(_), "ImportGrammar")
                | (Command::ImportPdf(_), "ImportPdf")
                | (Command::Ingest(_), "Ingest")
                | (Command::Learn(_), "Learn")
                | (Command::ListLanguages, "ListLanguages")
                | (Command::Nback(_), "Nback")
                | (Command::PrefetchMedia(_), "PrefetchMedia")
                | (Command::ExtractPhrases(_), "ExtractPhrases")
                | (Command::TranslatePhrases(_), "TranslatePhrases")
                | (Command::ExtractWords(_), "ExtractWords")
                | (Command::PromoteWords(_), "PromoteWords")
                | (Command::Preview(_), "Preview")
                | (Command::Quiz(_), "Quiz")
                | (Command::SeedGrammar(_), "SeedGrammar")
                | (Command::Sync(_), "Sync")
        )
    }

    #[test]
    fn prefetch_media_accepts_bounded_range_and_media_switches() -> Result<(), clap::Error> {
        let cli = Cli::try_parse_from([
            "wisecrow",
            "prefetch-media",
            "-n",
            "en",
            "-f",
            "br",
            "--limit",
            "100",
            "--offset",
            "300",
            "--max-bytes",
            "1048576",
            "--audio=false",
            "--images=true",
            "--dry-run",
        ])?;
        let Command::PrefetchMedia(args) = cli.command else {
            panic!("expected prefetch-media");
        };
        assert_eq!(
            (args.limit, args.offset, args.max_bytes, args.dry_run),
            (100, 300, 1_048_576, true)
        );
        assert_eq!((args.audio, args.images), (false, true));
        let defaults = Cli::try_parse_from(["wisecrow", "prefetch-media", "-n", "en", "-f", "fr"])?;
        let Command::PrefetchMedia(args) = defaults.command else {
            panic!("expected prefetch-media");
        };
        assert_eq!(
            (args.limit, args.offset, args.max_bytes, args.dry_run),
            (100, 0, 67_108_864, false)
        );
        assert_eq!((args.audio, args.images), (true, true));
        Ok(())
    }

    #[test]
    fn local_tmx_alias_requires_file() {
        assert!(Cli::try_parse_from([
            "wisecrow",
            "ingest",
            "-n",
            "en",
            "-f",
            "zh",
            "--tmx-target-lang",
            "zh_CN",
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "wisecrow",
            "ingest",
            "-n",
            "en",
            "-f",
            "zh",
            "--file",
            "sample.tmx",
            "--tmx-target-lang",
            "zh_CN",
        ])
        .is_ok());
    }

    #[rstest]
    #[case(&["wisecrow", "download", "-n", "en", "-f", "es"], "Download")]
    #[case(&["wisecrow", "d", "-n", "en", "-f", "fr"], "Download")]
    #[case(&["wisecrow", "ingest", "-n", "en", "-f", "es"], "Ingest")]
    #[case(&["wisecrow", "i", "-n", "ja", "-f", "en"], "Ingest")]
    #[case(&["wisecrow", "list-languages"], "ListLanguages")]
    #[case(&["wisecrow", "l"], "ListLanguages")]
    #[case(&["wisecrow", "seed-grammar", "--lang", "es", "--levels", "A1,A2"], "SeedGrammar")]
    #[case(&["wisecrow", "sg", "--lang", "es", "--levels", "A1"], "SeedGrammar")]
    #[case(&["wisecrow", "import-grammar", "--lang", "es", "--file", "rules.json"], "ImportGrammar")]
    #[case(&["wisecrow", "ig", "--lang", "es", "--file", "rules.json"], "ImportGrammar")]
    #[case(&["wisecrow", "import-pdf", "--lang", "es", "--level", "B1", "--file", "g.pdf"], "ImportPdf")]
    #[case(&["wisecrow", "ip", "--lang", "es", "--level", "B1", "--file", "g.pdf"], "ImportPdf")]
    #[case(&["wisecrow", "sync", "--remote", "https://example.com"], "Sync")]
    #[case(&["wisecrow", "s", "--remote", "https://example.com"], "Sync")]
    #[case(&["wisecrow", "generate-exercises", "--lang", "es", "--level", "B1"], "GenerateExercises")]
    #[case(&["wisecrow", "ge", "--lang", "es", "--level", "B1"], "GenerateExercises")]
    #[case(&["wisecrow", "frequency", "--lang", "es"], "Frequency")]
    #[case(&["wisecrow", "fr", "--lang", "es", "--file", "es_50k.txt"], "Frequency")]
    #[case(&["wisecrow", "nback", "-n", "en", "-f", "es"], "Nback")]
    #[case(&["wisecrow", "nb", "-n", "en", "-f", "de", "--mode", "word_translation"], "Nback")]
    #[case(&["wisecrow", "prefetch-media", "-n", "en", "-f", "es"], "PrefetchMedia")]
    #[case(&["wisecrow", "extract-phrases", "-l", "gd"], "ExtractPhrases")]
    #[case(
        &["wisecrow", "extract-words", "-n", "en", "-f", "gd", "--limit", "500", "--min-occurrences", "5"],
        "ExtractWords"
    )]
    #[case(
        &["wisecrow", "promote-words", "-n", "en", "-f", "gd", "--limit", "200", "--mode", "pending"],
        "PromoteWords"
    )]
    #[case(
        &["wisecrow", "promote-words", "-n", "en", "-f", "gd", "--limit", "200", "--mode", "retry-failed"],
        "PromoteWords"
    )]
    #[case(
        &["wisecrow", "translate-phrases", "-l", "gd", "-n", "en", "--limit", "50", "--refresh"],
        "TranslatePhrases"
    )]
    #[case(&["wisecrow", "pm", "-n", "en", "-f", "de"], "PrefetchMedia")]
    #[case(&["wisecrow", "gloss", "--sentence", "Меня зовут Иван", "--lang", "ru"], "Gloss")]
    #[case(&["wisecrow", "gl", "--sentence", "casa", "--lang", "es"], "Gloss")]
    #[case(&["wisecrow", "graded-reader", "-n", "en", "-f", "es", "--cefr", "B1"], "GradedReader")]
    #[case(&["wisecrow", "gr", "-n", "en", "-f", "es", "--cefr", "A2"], "GradedReader")]
    #[case(&["wisecrow", "gr", "-n", "en", "-f", "es", "--cefr", "A2", "--seed-states", "2,3"], "GradedReader")]
    #[case(&["wisecrow", "preview", "--file", "ep.srt", "-n", "en", "-f", "es"], "Preview")]
    #[case(&["wisecrow", "pv", "--file", "ep.vtt", "-n", "en", "-f", "es", "--unknown-only"], "Preview")]
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

    #[rstest]
    #[case(
        &["wisecrow", "download", "-n", "en", "-f", "es", "--corpus", "cc_matrix nllb"],
        Some(vec!["cc_matrix", "nllb"]),
        102_400
    )]
    #[case(
        &["wisecrow", "download", "-n", "en", "-f", "es"],
        None,
        102_400
    )]
    fn download_field_defaults(
        #[case] args: &[&str],
        #[case] expected_corpus: Option<Vec<&str>>,
        #[case] expected_max_size: u64,
    ) {
        let cli = Cli::parse_from(args);
        if let Command::Download(cmd_args) = cli.command {
            let corpus_strs: Option<Vec<&str>> = cmd_args
                .corpus
                .as_ref()
                .map(|v| v.iter().map(String::as_str).collect());
            assert_eq!(corpus_strs, expected_corpus);
            assert_eq!(cmd_args.max_file_size_mb, expected_max_size);
        } else {
            panic!("Expected Download command");
        }
    }

    proptest! {
        #[test]
        fn arbitrary_string_matches_iff_known(s in "\\PC{0,10}") {
            let is_known = SUPPORTED_LANGUAGE_INFO.iter().any(|(c, _)| *c == s);
            prop_assert_eq!(is_supported_language(&s), is_known);
        }
    }

    #[test]
    fn parses_user_and_sync_client_subcommands() {
        assert!(matches!(
            Cli::parse_from([
                "wisecrow",
                "user",
                "add",
                "--email",
                "a@b.c",
                "--display-name",
                "A",
                "--admin",
            ])
            .command,
            Command::User {
                command: UserCmd::Add(_)
            }
        ));
        assert!(matches!(
            Cli::parse_from(["wisecrow", "u", "list"]).command,
            Command::User {
                command: UserCmd::List
            }
        ));
        assert!(matches!(
            Cli::parse_from(["wisecrow", "sync-client", "add", "--name", "laptop"]).command,
            Command::SyncClient {
                command: SyncClientCmd::Add(_)
            }
        ));
        assert!(matches!(
            Cli::parse_from(["wisecrow", "sc", "revoke", "--name", "laptop"]).command,
            Command::SyncClient {
                command: SyncClientCmd::Revoke(_)
            }
        ));
    }
}
