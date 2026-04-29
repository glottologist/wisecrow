# wisecrow-core

`wisecrow-core` (crate name: `wisecrow`) is the library and binary at the
heart of the workspace. This page lists the public types per module and links
to the source with `file:line` references so you can jump straight to the
declaration.

> The CLI binary is `src/bin/wisecrow.rs`. Everything below is reachable as a
> library too.

## Module map

| Module | What it does |
|--------|--------------|
| [`config`](#config) | Environment-driven configuration with secret-zeroing strings. |
| [`cli`](#cli) | clap-derived command surface and the supported-language whitelist. |
| [`errors`](#errors) | The crate-wide `WisecrowError`. |
| [`files`](#files) | OPUS URL construction. |
| [`downloader`](#downloader) | HTTP fetch with retry + decompression. |
| [`ingesting`](#ingesting) | Parser and persister for TMX and OPUS XML. |
| [`srs`](#srs) | FSRS scheduler and session lifecycle. |
| [`vocabulary`](#vocabulary) | Unlearned-words query. |
| [`users`](#users) | Users repository. |
| [`dnb`](#dnb) | Dual n-back engine and adaptation. |
| [`grammar`](#grammar) | CEFR rules, AI seeding, quizzes, PDF import. |
| [`llm`](#llm) | Provider trait + Anthropic/OpenAI implementations. |
| [`media`](#media) | Audio + image cache and prefetch. |
| [`sync`](#sync) | Remote sync client. |
| [`tui`](#tui) | ratatui flashcard runner and quiz. |
| [`frequency`](#frequency) | Hermit Dave frequency-list import. |
| [`dto_convert`](#dto_convert) | Domain → DTO conversions. |

## Top-level types

Defined directly in `lib.rs`:

```rust,ignore
pub struct Native(String);
pub struct Foreign(String);
pub struct Langs { native: Native, foreign: Foreign }

impl Langs {
    pub fn new(native: impl Into<String>, foreign: impl Into<String>) -> Self;
    pub fn native_code(&self) -> &str;
    pub fn foreign_code(&self) -> &str;
}
```

The newtype split prevents accidentally swapping native/foreign codes —
they have distinct types at the call site.
Source: `wisecrow-core/src/lib.rs:21`.

## `config`

```rust,ignore
pub struct SecureString(String);
pub struct Config {
    pub db_url: Option<SecureString>,
    pub db_address: Option<String>,
    pub db_name: Option<String>,
    pub db_user: Option<String>,
    pub db_password: Option<SecureString>,
    pub unsplash_api_key: Option<SecureString>,
    pub llm_provider: Option<String>,
    pub llm_api_key: Option<SecureString>,
    pub remote_url: Option<String>,
    pub remote_api_key: Option<SecureString>,
    pub sync_api_key: Option<SecureString>,
}

impl SecureString { pub fn expose(&self) -> &str; }
impl Config { pub fn database_url(&self) -> Result<Cow<'_, str>, WisecrowError>; }
```

`SecureString` derives `Zeroize` and `ZeroizeOnDrop` and never implements
`Debug` for its content. URL assembly goes through `url::Url`; format-string
URL construction is forbidden by the project guidelines.
Source: `wisecrow-core/src/config.rs:9`.

## `cli`

```rust,ignore
pub const SUPPORTED_LANGUAGE_INFO: &[(&str, &str)];
pub fn is_supported_language(code: &str) -> bool;

pub struct Cli { pub command: Command }
pub enum Command { Download(LanguageArgs), Ingest(LanguageArgs), Learn(LearnArgs), Nback(NbackArgs), ... }
```

Each `*Args` struct is a `clap::Args` — see the
[CLI reference](../reference/cli-reference.md) for argument tables.
Source: `wisecrow-core/src/cli.rs`.

## `errors`

```rust,ignore
#[derive(thiserror::Error, Debug)]
pub enum WisecrowError {
    DownloadRetriesExhausted,
    UnableToParseUrl(#[from] url::ParseError),
    UnableToGetFile(#[from] reqwest::Error),
    UnableToCreateFile(#[from] std::io::Error),
    UnableToConstructProgressBarStyle(#[from] indicatif::style::TemplateError),
    PersistenceMigrationError(#[from] sqlx::migrate::MigrateError),
    PersistenceConnectionError(#[from] sqlx::Error),
    ConfigurationError(String),
    InvalidInput(String),
    MediaError(String),
    PdfExtractionError(String),
    QuizGenerationError(String),
    LlmError(String),
    SyncError(String),
}
```

Every fallible function in the crate returns `Result<T, WisecrowError>`.
Source: `wisecrow-core/src/errors.rs:5`.

## `files`

```rust,ignore
pub enum Corpus { OpenSubtitles, CcMatrix, Nllb }
pub enum Compression { GzCompressed, ZipCompressed }
pub struct LanguageFileInfo { pub corpus, pub target_location, pub file_name, pub compressed }
pub struct LanguageFiles { pub files: Vec<LanguageFileInfo> }

impl Corpus { /* TryFrom<&str> */ }
impl LanguageFiles {
    pub fn new(langs: &Langs, corpora: Option<&[Corpus]>) -> Result<Self, WisecrowError>;
}
```

URL construction normalises the lexical order of language codes (so
`en`-`es` and `es`-`en` map to the same OPUS file).
Source: `wisecrow-core/src/files.rs`.

## `downloader`

```rust,ignore
#[derive(Clone, Copy)]
pub struct DownloadConfig {
    pub max_retries: u32,
    pub timeout_seconds: u64,
    pub max_file_size_mb: u64,
    pub unpack: bool,
}

pub struct Downloader { /* HTTP client + config */ }

impl Downloader {
    pub fn new(config: DownloadConfig) -> Result<Self, WisecrowError>;
    pub async fn download(&self, file: &LanguageFileInfo) -> Result<String, WisecrowError>;
    pub async fn download_to(&self, file: &LanguageFileInfo, output_dir: Option<&Path>) -> Result<String, WisecrowError>;
}
```

The downloader caps decompression at 1 GiB, rejects ZIP entries with
suspicious paths, and exponentially backs off across retries
(`2^attempt` seconds).
Source: `wisecrow-core/src/downloader.rs`.

## `ingesting`

```rust,ignore
pub struct Ingester { /* PgPool + DownloadConfig */ }
impl Ingester {
    pub async fn download_only(config: &DownloadConfig, file: &LanguageFileInfo) -> Result<String, WisecrowError>;
    pub async fn download_to_dir(config: &DownloadConfig, file: &LanguageFileInfo, output_dir: &Path) -> Result<String, WisecrowError>;
    pub async fn download_and_ingest(&self, file: &LanguageFileInfo, native_lang: &str, foreign_lang: &str) -> Result<(), WisecrowError>;
    pub async fn ingest_from_file(&self, path: &str, file: &LanguageFileInfo, native_lang: &str, foreign_lang: &str) -> Result<(), WisecrowError>;
    pub fn spawn(pool: PgPool, config: DownloadConfig, langs: &Langs, file: LanguageFileInfo) -> JoinHandle<()>;
}
```

### Submodules

```rust,ignore
pub mod parsing {
    pub struct TranslationPair { pub source_text: String, pub target_text: String }
    pub struct CorpusParser;
    impl CorpusParser {
        pub async fn parse_tmx_file(path: &str, source_lang: &str, target_lang: &str, sender: &Sender<TranslationPair>) -> Result<usize, WisecrowError>;
        pub async fn parse_xml_alignment_file(...) -> Result<usize, WisecrowError>;
    }
}

pub mod persisting {
    pub struct DatabasePersister { /* PgPool */ }
    impl DatabasePersister {
        pub fn new(pool: PgPool) -> Self;
        pub async fn ensure_language(&self, code: &str, name: &str) -> Result<i32, WisecrowError>;
        pub async fn persist_translations(&self, batch: &[TranslationPair], from: i32, to: i32) -> Result<(), WisecrowError>;
        pub async fn consume(&self, receiver: Receiver<TranslationPair>, from: i32, to: i32) -> Result<(), WisecrowError>;
    }
}
```

Source: `wisecrow-core/src/ingesting/`.

## `srs`

```rust,ignore
pub mod scheduler {
    pub enum CardStatus { New, Learning, Review, Relearning }
    pub enum ReviewRating { Again, Hard, Good, Easy }
    pub struct CardState { /* card_id, translation_id, phrases, FSRS fields */ }
    pub struct CardManager;

    impl CardManager {
        pub async fn ensure_cards(pool: &PgPool, translation_ids: &[i32]) -> Result<Vec<i32>, WisecrowError>;
        pub async fn get_card_by_id(pool: &PgPool, card_id: i32) -> Result<CardState, WisecrowError>;
        pub async fn due_cards(pool: &PgPool, native: &str, foreign: &str, limit: u32) -> Result<Vec<CardState>, WisecrowError>;
        pub async fn review(pool: &PgPool, card: &CardState, rating: ReviewRating) -> Result<CardState, WisecrowError>;
        pub async fn card_for_translation(pool: &PgPool, translation_id: i32) -> Result<Option<CardState>, WisecrowError>;
    }
}

pub mod session {
    pub struct Session { pub id, pub native_lang, pub foreign_lang, pub deck_size, pub speed_ms, pub current_index, pub cards }
    pub struct SessionManager;

    impl SessionManager {
        pub async fn create(pool, user_id, native, foreign, deck_size, speed_ms) -> Result<Session, WisecrowError>;
        pub async fn resume(pool, user_id, native, foreign) -> Result<Option<Session>, WisecrowError>;
        pub async fn pause(pool, session_id) -> Result<(), WisecrowError>;
        pub async fn complete(pool, session_id) -> Result<(), WisecrowError>;
        pub async fn answer_card(pool, session_id, card, rating) -> Result<CardState, WisecrowError>;
    }
}
```

`due_cards` priority is **Relearning > Learning > New > Review**, then by
`due ASC`. The narrowing helper `f64_to_f32_clamped` ensures FSRS's `f64`
state never produces NaN or Infinity in `REAL` columns.
Source: `wisecrow-core/src/srs/`.

## `vocabulary`

```rust,ignore
pub struct VocabularyEntry { pub translation_id, pub from_phrase, pub to_phrase, pub frequency }
pub struct VocabularyQuery;

impl VocabularyQuery {
    pub async fn unlearned(pool: &PgPool, native: &str, foreign: &str, limit: u32) -> Result<Vec<VocabularyEntry>, WisecrowError>;
}
```

The query filters on `frequency > 1` and `LENGTH(phrase) BETWEEN 2 AND 200` so
single-letter rows and runaway TMX glitches do not pollute decks.
Source: `wisecrow-core/src/vocabulary.rs`.

## `users`

```rust,ignore
pub struct User { pub id, pub display_name, pub created_at }
pub const DEFAULT_USER_ID: i32 = 1;
pub struct UserRepository;

impl UserRepository {
    pub async fn create(pool: &PgPool, display_name: &str) -> Result<User, WisecrowError>;
    pub async fn get_by_id(pool: &PgPool, id: i32) -> Result<Option<User>, WisecrowError>;
    pub async fn list_all(pool: &PgPool) -> Result<Vec<User>, WisecrowError>;
}
```

Source: `wisecrow-core/src/users.rs`.

## `dnb`

```rust,ignore
pub enum DnbMode { AudioWritten, WordTranslation, AudioImage }
pub struct DnbConfig { pub mode, pub n_level, pub interval_ms }
pub struct DnbVocab { pub translation_id, pub from_phrase, pub to_phrase }
pub struct Trial { pub trial_number, pub n_level, pub audio_vocab, pub visual_vocab, pub audio_match, pub visual_match, pub interval_ms }
pub struct TrialResponse { pub audio_response, pub visual_response, pub response_time_ms }
pub struct CompletedTrial { pub trial: Trial, pub response: TrialResponse }
pub struct DnbEngine { /* state machine */ }

impl DnbEngine {
    pub fn new(vocab: Vec<DnbVocab>, config: &DnbConfig, seed: u64) -> Result<Self, WisecrowError>;
    pub fn next_trial(&mut self) -> Trial;
    pub fn record_response(&mut self, response: TrialResponse);
    pub fn should_terminate(&self) -> bool;
    pub fn state(&self) -> &AdaptationState;
    pub fn completed_trials(&self) -> &[CompletedTrial];
}
```

### Submodules

- `scoring` — `AdaptationState`, `apply_adaptation`, `should_terminate`,
  `channel_accuracy`, `Channel`.
- `session` — `DnbSessionRepository::{create_session, save_trial, complete_session, load_vocab}`.
- `feedback` — `apply_srs_feedback(pool, trials)` writes back into the SRS.

The match probability is `0.30`. The minimum vocabulary pool is `8`. Both
constants are deliberately out-of-bound for tuning; change them in source
only if you understand the n-back literature.
Source: `wisecrow-core/src/dnb/`.

## `grammar`

```rust,ignore
pub mod rules {
    pub struct CefrLevel; pub enum RuleSource { Manual, Ai, Pdf }
    pub struct GrammarRule; pub struct RuleExample;
    pub struct NewGrammarRule; pub struct NewRuleExample;
    pub struct RuleRepository;

    impl RuleRepository {
        pub async fn upsert_rule(pool, language_id, cefr_level_id, &NewGrammarRule) -> Result<i32, WisecrowError>;
        pub async fn rules_for_level(pool, language_id, cefr_level_code) -> Result<Vec<GrammarRule>, WisecrowError>;
        pub async fn ensure_cefr_level(pool, code) -> Result<i32, WisecrowError>;
        pub async fn count_rules(pool, language_id) -> Result<i64, WisecrowError>;
    }

    pub async fn import_from_json(pool, language_id, path) -> Result<usize, WisecrowError>;
    pub async fn import_from_pdf(pool, language_id, cefr_level_code, pdf_path) -> Result<usize, WisecrowError>;
}

pub mod seeder { pub async fn seed_grammar(...) -> Result<usize, WisecrowError>; }
pub mod ai_exercises { pub async fn generate_exercises(...) -> Result<(Vec<ClozeQuiz>, Vec<MultipleChoiceQuiz>), WisecrowError>; }
pub mod quiz { pub struct ClozeQuiz; pub struct MultipleChoiceQuiz; }
pub mod pdf { pub fn extract(path: &Path) -> Result<ExtractedContent, WisecrowError>; }
```

Source: `wisecrow-core/src/grammar/`.

## `llm`

```rust,ignore
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn generate(&self, prompt: &str, max_tokens: u32) -> Result<String, WisecrowError>;
    fn name(&self) -> &str;
}

pub fn create_provider(config: &Config) -> Result<Box<dyn LlmProvider>, WisecrowError>;
```

Two implementations are bundled:

- `anthropic::AnthropicProvider` — Claude Sonnet 4 via the messages API.
- `openai::OpenAiProvider` — GPT-4o via the chat completions API.

Models and endpoints are constants at the top of each file; bump them in
source if you need a different model.
Source: `wisecrow-core/src/llm/`.

## `media`

```rust,ignore
pub enum MediaType { Audio, Image }
pub struct MediaContext { pub cache, pub http_client, pub foreign_lang, pub unsplash_api_key }
pub struct MediaCache { /* cache_dir + PgPool */ }

impl MediaCache {
    pub fn new(pool: PgPool) -> Result<Self, WisecrowError>;
    pub async fn get_or_fetch<F, Fut>(&self, translation_id: i32, media_type: MediaType, fetcher: F) -> Result<PathBuf, WisecrowError>
        where F: FnOnce() -> Fut,
              Fut: Future<Output = Result<Vec<u8>, WisecrowError>>;
}
```

Audio (`audio` feature) goes through Microsoft Edge TTS via `msedge-tts`;
images (`images` feature) hit Unsplash.
Source: `wisecrow-core/src/media/`.

## `sync`

```rust,ignore
pub async fn run_sync(pool: &PgPool, remote_url: &str, api_key: Option<&str>) -> Result<(), WisecrowError>;

pub mod client {
    pub struct SyncClient;
    impl SyncClient {
        pub fn new(remote_url: &str, api_key: Option<&str>) -> Result<Self, WisecrowError>;
        pub async fn sync_languages(&self, pool: &PgPool) -> Result<usize, WisecrowError>;
        pub async fn sync_translations(&self, pool: &PgPool) -> Result<usize, WisecrowError>;
        pub async fn sync_grammar_rules(&self, pool: &PgPool) -> Result<usize, WisecrowError>;
    }
}
```

Source: `wisecrow-core/src/sync/`.

## `tui`

The TUI layer is a small pile of `ratatui` glue:

- `app::run_tui(pool, session, media_ctx)` drives the flashcard runner.
- `quiz::run_quiz(pdf_path, num_questions)` runs a one-shot quiz from a PDF.
- `speed::SpeedController` tracks the auto-advance timer.
- `widgets::card`, `widgets::stats` render card faces and footer stats.

Source: `wisecrow-core/src/tui/`.

## `frequency`

```rust,ignore
pub struct FrequencyUpdater;
impl FrequencyUpdater {
    pub async fn update_from_hermit_dave(pool: &PgPool, lang_code: &str) -> Result<usize, WisecrowError>;
    pub async fn update_from_file(pool: &PgPool, lang_code: &str, path: &str) -> Result<usize, WisecrowError>;
}
```

Updates `translations.frequency` in batches of 1000 using
`UPDATE … FROM unnest($1, $2)`. Used to overlay external frequency lists when
a corpus alone is too noisy.
Source: `wisecrow-core/src/frequency.rs`.

## `dto_convert`

`From` impls and helper functions that bridge domain types in `wisecrow-core`
to serde DTOs in `wisecrow-dto`. Notable helpers:

- `script_direction_for(code) -> ScriptDirection` (RTL detection)
- `language_info(code, name) -> LanguageInfo`
- `grammar_rule_to_dto(&GrammarRule, cefr_level_code) -> GrammarRuleDto`
- `quizzes_to_dto(&[ClozeQuiz], &[MultipleChoiceQuiz]) -> Vec<QuizItemDto>`
- `adaptation_to_dto(&AdaptationState, &[CompletedTrial], terminated) -> DnbAdaptationDto`

Source: `wisecrow-core/src/dto_convert.rs`.
