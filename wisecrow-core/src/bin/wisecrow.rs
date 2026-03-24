use anyhow::Error;
use clap::Parser;
use config::{Config as ConfigLoader, Environment};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;
use std::str::FromStr;
use tokio::{
    select,
    signal::unix::{signal, SignalKind},
    task::JoinHandle,
};
use tracing::{error, info};
use wisecrow::{
    cli::{
        is_supported_language, Cli, Command, LanguageArgs, LearnArgs, QuizArgs,
        SUPPORTED_LANGUAGE_INFO,
    },
    config::Config,
    downloader::DownloadConfig,
    errors::WisecrowError,
    files::{Corpus, LanguageFileInfo, LanguageFiles},
    ingesting::Ingester,
    media::MediaContext,
    srs::session::SessionManager,
    tui::{app, quiz},
    Langs,
};

const MAX_DB_CONNECTIONS: u32 = 5;

async fn assure_db(database_url: &str) -> Result<PgPool, WisecrowError> {
    let connect_options = PgConnectOptions::from_str(database_url)?;
    let pool = PgPoolOptions::new()
        .max_connections(MAX_DB_CONNECTIONS)
        .connect_with(connect_options)
        .await
        .map_err(WisecrowError::PersistenceConnectionError)?;
    info!("Connected to database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .map_err(WisecrowError::PersistenceMigrationError)?;
    info!("Database migrations applied");
    Ok(pool)
}

fn abort_all(handles: &[JoinHandle<()>]) {
    for handle in handles {
        handle.abort();
    }
}

async fn wait_for_shutdown_signal(term_signal: &mut tokio::signal::unix::Signal) {
    select! {
        _ = tokio::signal::ctrl_c() => info!("Received Ctrl+C, shutting down"),
        _ = term_signal.recv() => info!("Received SIGTERM, shutting down"),
    }
}

async fn run_until_done_or_signal(mut handles: Vec<JoinHandle<()>>) -> Result<(), Error> {
    let mut term_signal = signal(SignalKind::terminate())?;

    loop {
        let Some(last) = handles.last_mut() else {
            info!("All tasks completed");
            return Ok(());
        };
        select! {
            result = last => {
                handles.pop();
                if let Err(e) = result {
                    error!("Task panicked: {e}");
                }
            }
            () = wait_for_shutdown_signal(&mut term_signal) => {
                abort_all(&handles);
                return Ok(());
            }
        }
    }
}

async fn load_config_and_pool() -> Result<(Config, PgPool), WisecrowError> {
    let settings = ConfigLoader::builder()
        .add_source(Environment::with_prefix("WISECROW").separator("__"))
        .build()
        .map_err(|e| WisecrowError::ConfigurationError(e.to_string()))?;
    let config: Config = settings
        .try_deserialize()
        .map_err(|e| WisecrowError::ConfigurationError(e.to_string()))?;
    let database_url = config.database_url()?;
    let pool = assure_db(&database_url).await?;
    Ok((config, pool))
}

fn validate_languages(native: &str, foreign: &str) -> Result<(), WisecrowError> {
    if !is_supported_language(native) {
        return Err(WisecrowError::InvalidInput(format!(
            "Unsupported native language: {native}"
        )));
    }
    if !is_supported_language(foreign) {
        return Err(WisecrowError::InvalidInput(format!(
            "Unsupported foreign language: {foreign}"
        )));
    }
    if native == foreign {
        return Err(WisecrowError::InvalidInput(
            "Native and foreign languages must be different".to_owned(),
        ));
    }
    Ok(())
}

fn parse_corpora(args: Option<&[String]>) -> Result<Option<Vec<Corpus>>, WisecrowError> {
    args.map(|v| v.iter().map(|s| Corpus::try_from(s.as_str())).collect())
        .transpose()
}

struct PreparedJob {
    langs: Langs,
    files: Vec<LanguageFileInfo>,
    download_config: DownloadConfig,
}

fn prepare_job(args: LanguageArgs) -> Result<PreparedJob, WisecrowError> {
    validate_languages(&args.native_lang, &args.foreign_lang)?;
    let corpora = parse_corpora(args.corpus.as_deref())?;
    let langs = Langs::new(args.native_lang, args.foreign_lang);
    let files = LanguageFiles::new(&langs, corpora.as_deref())?;
    let download_config = DownloadConfig {
        max_file_size_mb: args.max_file_size_mb,
        unpack: args.unpack,
        ..Default::default()
    };
    Ok(PreparedJob {
        langs,
        files: files.files,
        download_config,
    })
}

async fn handle_download(args: LanguageArgs) -> Result<(), Error> {
    let job = prepare_job(args)?;
    let mut handles = Vec::new();
    for file in job.files {
        let cfg = job.download_config;
        handles.push(tokio::spawn(async move {
            if let Err(e) = Ingester::download_only(&cfg, &file).await {
                error!("Download failed for {}: {e:?}", file.file_name);
            }
        }));
    }
    run_until_done_or_signal(handles).await
}

async fn handle_ingest(args: LanguageArgs) -> Result<(), Error> {
    let job = prepare_job(args)?;
    let (_config, pool) = load_config_and_pool().await?;

    let mut handles = Vec::new();
    for file in job.files {
        handles.push(Ingester::spawn(
            pool.clone(), // clone: PgPool is Arc-based
            job.download_config,
            &job.langs,
            file,
        ));
    }
    run_until_done_or_signal(handles).await
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt::init();
    if let Err(e) = dotenv::dotenv() {
        tracing::debug!("No .env file loaded: {e}");
    }
    let cli = Cli::parse();

    match cli.command {
        Command::Download(args) => handle_download(args).await?,
        Command::Ingest(args) => handle_ingest(args).await?,
        Command::Learn(args) => handle_learn(args).await?,
        Command::ListLanguages => {
            println!("{:<10} Language", "Code");
            println!("{}", "-".repeat(40));
            for (code, name) in SUPPORTED_LANGUAGE_INFO {
                println!("{code:<10} {name}");
            }
        }
        Command::Quiz(args) => handle_quiz(args)?,
    }
    Ok(())
}

async fn handle_learn(args: LearnArgs) -> Result<(), Error> {
    validate_languages(&args.native_lang, &args.foreign_lang)?;
    let (config, pool) = load_config_and_pool().await?;

    let session = match SessionManager::resume(&pool, &args.native_lang, &args.foreign_lang).await?
    {
        Some(session) => {
            info!(
                "Resuming session {} at card {}/{}",
                session.id,
                session.current_index,
                session.cards.len()
            );
            session
        }
        None => {
            info!(
                "Creating new session: {} -> {}, deck_size={}, speed={}ms",
                args.native_lang, args.foreign_lang, args.deck_size, args.speed_ms
            );
            SessionManager::create(
                &pool,
                &args.native_lang,
                &args.foreign_lang,
                args.deck_size,
                args.speed_ms,
            )
            .await?
        }
    };

    if session.cards.is_empty() {
        info!("No cards available. Ingest some data first with `wisecrow ingest`.");
        return Ok(());
    }

    let media_ctx = match MediaContext::new(
        pool.clone(), // clone: PgPool is Arc-based
        args.foreign_lang,
        config.unsplash_api_key,
    ) {
        Ok(ctx) => Some(ctx),
        Err(e) => {
            tracing::warn!("Media cache init failed, running without media: {e}");
            None
        }
    };

    app::run_tui(pool, session, media_ctx).await?;
    Ok(())
}

fn handle_quiz(args: QuizArgs) -> Result<(), Error> {
    let path = std::path::Path::new(&args.pdf_path);
    if !path.exists() {
        return Err(
            WisecrowError::InvalidInput(format!("PDF file not found: {}", args.pdf_path)).into(),
        );
    }
    quiz::run_quiz(path, args.num_questions)?;
    Ok(())
}
