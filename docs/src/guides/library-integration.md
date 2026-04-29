# Library integration

`wisecrow-core` is a library before it is a binary. This guide shows how to
embed Wisecrow's behaviours directly in another Rust application.

## Add the dependency

If you are working inside the workspace:

```toml
# Cargo.toml of your crate
[dependencies]
wisecrow = { path = "../wisecrow-core" }   # the library is named `wisecrow`
wisecrow-dto = { path = "../wisecrow-dto" }
sqlx = { version = "0.7", features = ["postgres", "runtime-tokio-rustls"] }
tokio = { version = "1", features = ["full"] }
```

If you are consuming the crate from elsewhere, point to the git URL or
publish a local fork — Wisecrow is not currently published to crates.io.

## Bootstrap a pool

Wisecrow's binary uses a 5-connection pool with embedded migrations:

```rust,ignore
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use std::str::FromStr;

const MAX_DB_CONNECTIONS: u32 = 5;

async fn open_pool(url: &str) -> sqlx::Result<PgPool> {
    let opts = PgConnectOptions::from_str(url)?;
    let pool = PgPoolOptions::new()
        .max_connections(MAX_DB_CONNECTIONS)
        .connect_with(opts)
        .await?;
    sqlx::migrate!("../wisecrow-core/migrations")
        .run(&pool)
        .await?;
    Ok(pool)
}
```

When you run from the binary, the migration path is the relative
`./migrations` directory next to `wisecrow-core`. From a sibling crate, point
the macro at the canonical location or vendor the SQL files into your own
crate.

## Ingest from inside a service

```rust,ignore
use wisecrow::downloader::DownloadConfig;
use wisecrow::files::{Corpus, LanguageFiles};
use wisecrow::ingesting::Ingester;
use wisecrow::Langs;

async fn ingest_pair(pool: sqlx::PgPool, native: &str, foreign: &str) -> anyhow::Result<()> {
    let langs = Langs::new(native, foreign);
    let files = LanguageFiles::new(&langs, Some(&[Corpus::OpenSubtitles]))?;
    let cfg = DownloadConfig::default();

    let mut handles = Vec::new();
    for file in files.files {
        handles.push(Ingester::spawn(pool.clone(), cfg, &langs, file));
    }
    for h in handles { let _ = h.await; }
    Ok(())
}
```

If you want fine-grained progress reporting, drop down to
`Ingester::download_and_ingest` and orchestrate the joins yourself.

## Schedule reviews

```rust,ignore
use wisecrow::srs::scheduler::{CardManager, ReviewRating};

async fn answer(pool: &sqlx::PgPool, card_id: i32, rating: ReviewRating) -> anyhow::Result<()> {
    let card = CardManager::get_card_by_id(pool, card_id).await?;
    let _next = CardManager::review(pool, &card, rating).await?;
    Ok(())
}
```

`CardManager::review` is the only function you need to drive the FSRS
state machine — it computes the next state, persists it, and returns the
fresh `CardState`.

## Run the dual n-back engine headlessly

Useful for analytics or a custom UI:

```rust,ignore
use wisecrow::dnb::{DnbConfig, DnbEngine, DnbMode, TrialResponse};
use wisecrow::dnb::session::DnbSessionRepository;

async fn play_one_session(pool: &sqlx::PgPool, user_id: i32) -> anyhow::Result<()> {
    let vocab = DnbSessionRepository::load_vocab(pool, "en", "es", 100).await?;
    let cfg = DnbConfig { mode: DnbMode::AudioWritten, n_level: 2, interval_ms: 4000 };
    let mut engine = DnbEngine::new(vocab, &cfg, /* seed = */ 42)?;

    while !engine.should_terminate() {
        let _trial = engine.next_trial();
        // present `_trial` to the user, gather a response...
        engine.record_response(TrialResponse {
            audio_response: Some(true),
            visual_response: Some(false),
            response_time_ms: Some(900),
        });
    }
    Ok(())
}
```

## Talk to the LLM

```rust,ignore
use wisecrow::config::Config;
use wisecrow::llm::create_provider;

let config: Config = /* loaded via your own configuration crate */ unimplemented!();
let provider = create_provider(&config)?;
let response = provider.generate("Translate: hello", 256).await?;
```

If you need a custom provider, implement
`#[async_trait] impl LlmProvider for MyProvider` and pass it directly to
the seeder/exercise functions instead of using the factory.

## Convert domain types to DTOs

`wisecrow-core` has `From` impls and helpers that map every domain type to
a `wisecrow-dto` type:

```rust,ignore
use wisecrow::srs::scheduler::CardState;
use wisecrow_dto::CardDto;

let dto = CardDto::from(&card_state);
```

For grammar rules use the helper:

```rust,ignore
use wisecrow::dto_convert::grammar_rule_to_dto;
let dto = grammar_rule_to_dto(&rule, "B1");
```

## Errors

Every fallible function returns `Result<T, WisecrowError>`. Pin your own
error type to it via `thiserror`:

```rust,ignore
#[derive(thiserror::Error, Debug)]
enum MyError {
    #[error(transparent)]
    Wisecrow(#[from] wisecrow::errors::WisecrowError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}
```

The `From` conversions on `WisecrowError` already cover `reqwest::Error`,
`url::ParseError`, `std::io::Error`, and `sqlx::Error` — you can usually
just propagate it.
