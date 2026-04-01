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
        is_supported_language, Cli, Command, DownloadAllArgs, GenerateExercisesArgs,
        ImportGrammarArgs, ImportPdfArgs, LanguageArgs, LearnArgs, NbackArgs, PrefetchMediaArgs,
        QuizArgs, SeedGrammarArgs, SyncArgs, SUPPORTED_LANGUAGE_INFO,
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

fn resolve_language_name(code: &str) -> Result<&'static str, WisecrowError> {
    SUPPORTED_LANGUAGE_INFO
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, n)| *n)
        .ok_or_else(|| WisecrowError::InvalidInput(format!("Unknown language: {code}")))
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

async fn handle_download_all(args: DownloadAllArgs) -> Result<(), Error> {
    if !is_supported_language(&args.native_lang) {
        return Err(WisecrowError::InvalidInput(format!(
            "Unsupported native language: {}",
            args.native_lang
        ))
        .into());
    }

    std::fs::create_dir_all(&args.output_dir)?;
    let root = std::path::Path::new(&args.output_dir)
        .canonicalize()
        .map_err(|e| WisecrowError::InvalidInput(format!("Invalid output directory: {e}")))?;
    let corpora = parse_corpora(args.corpus.as_deref())?;
    let download_config = DownloadConfig {
        max_file_size_mb: args.max_file_size_mb,
        unpack: args.unpack,
        ..Default::default()
    };

    let foreign_codes: Vec<&str> = SUPPORTED_LANGUAGE_INFO
        .iter()
        .map(|(code, _)| *code)
        .filter(|code| *code != args.native_lang)
        .collect();

    let total = foreign_codes.len();
    info!(
        "Downloading corpora for {} language pairs from {}",
        total, args.native_lang
    );

    for (idx, foreign) in foreign_codes.iter().enumerate() {
        let pair_dir = root.join(format!("{}-{foreign}", args.native_lang));
        if let Err(e) = std::fs::create_dir_all(&pair_dir) {
            error!("Failed to create directory {}: {e}", pair_dir.display());
            continue;
        }

        let langs = Langs::new(&args.native_lang, *foreign);
        let files = match LanguageFiles::new(&langs, corpora.as_deref()) {
            Ok(f) => f,
            Err(e) => {
                error!(
                    "Failed to build file list for {}-{foreign}: {e}",
                    args.native_lang
                );
                continue;
            }
        };

        info!(
            "[{}/{}] Downloading {} files for {}-{foreign}",
            idx.saturating_add(1),
            total,
            files.files.len(),
            args.native_lang,
        );

        let mut handles = Vec::new();
        for file in files.files {
            let cfg = download_config;
            let dir = pair_dir.clone();
            handles.push(tokio::spawn(async move {
                if let Err(e) = Ingester::download_to_dir(&cfg, &file, &dir).await {
                    error!("Download failed for {}: {e:?}", file.file_name);
                }
            }));
        }

        for handle in handles {
            if let Err(e) = handle.await {
                error!("Task panicked: {e}");
            }
        }
    }

    info!("Download-all complete");
    Ok(())
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

async fn handle_seed_grammar(args: SeedGrammarArgs) -> Result<(), Error> {
    let (config, pool) = load_config_and_pool().await?;
    let provider = wisecrow::llm::create_provider(&config)?;
    let lang_name = resolve_language_name(&args.lang)?;
    let level_refs: Vec<&str> = args.levels.iter().map(String::as_str).collect();

    let count = wisecrow::grammar::seeder::seed_grammar(
        &pool,
        provider.as_ref(),
        &args.lang,
        lang_name,
        &level_refs,
    )
    .await?;

    info!("Seeded {count} grammar rules");
    Ok(())
}

async fn handle_import_grammar(args: ImportGrammarArgs) -> Result<(), Error> {
    let (_config, pool) = load_config_and_pool().await?;
    let lang_name = resolve_language_name(&args.lang)?;

    let persister = wisecrow::ingesting::persisting::DatabasePersister::new(
        pool.clone(), // clone: PgPool is Arc-based
    );
    let language_id = persister.ensure_language(&args.lang, lang_name).await?;

    let path = std::path::Path::new(&args.file)
        .canonicalize()
        .map_err(|_| WisecrowError::InvalidInput(format!("File not found: {}", args.file)))?;

    let count = wisecrow::grammar::rules::import_from_json(&pool, language_id, &path).await?;
    info!("Imported {count} grammar rules from {}", args.file);
    Ok(())
}

async fn handle_import_pdf(args: ImportPdfArgs) -> Result<(), Error> {
    let (_config, pool) = load_config_and_pool().await?;
    let lang_name = resolve_language_name(&args.lang)?;

    let persister = wisecrow::ingesting::persisting::DatabasePersister::new(
        pool.clone(), // clone: PgPool is Arc-based
    );
    let language_id = persister.ensure_language(&args.lang, lang_name).await?;

    let path = std::path::Path::new(&args.file)
        .canonicalize()
        .map_err(|_| WisecrowError::InvalidInput(format!("File not found: {}", args.file)))?;

    let count =
        wisecrow::grammar::rules::import_from_pdf(&pool, language_id, &args.level, &path).await?;
    info!("Imported {count} grammar rules from PDF {}", args.file);
    Ok(())
}

async fn handle_sync(args: SyncArgs) -> Result<(), Error> {
    let (_config, pool) = load_config_and_pool().await?;
    wisecrow::sync::run_sync(&pool, &args.remote, args.api_key.as_deref()).await?;
    Ok(())
}

async fn handle_generate_exercises(args: GenerateExercisesArgs) -> Result<(), Error> {
    let (config, pool) = load_config_and_pool().await?;
    let provider = wisecrow::llm::create_provider(&config)?;

    let (cloze, mc) = wisecrow::grammar::ai_exercises::generate_exercises(
        &pool,
        provider.as_ref(),
        &args.lang,
        &args.level,
        args.count,
    )
    .await?;

    info!(
        "Generated {} cloze and {} multiple-choice exercises",
        cloze.len(),
        mc.len()
    );

    for (i, q) in cloze.iter().enumerate() {
        println!(
            "Cloze {}: {} [Answer: {}]",
            i.saturating_add(1),
            q.sentence_with_blank,
            q.answer
        );
    }
    for (i, q) in mc.iter().enumerate() {
        println!(
            "MC {}: {} [Correct: {}]",
            i.saturating_add(1),
            q.question,
            q.options[q.correct_index]
        );
    }

    Ok(())
}

async fn handle_prefetch_media(args: PrefetchMediaArgs) -> Result<(), Error> {
    validate_languages(&args.native_lang, &args.foreign_lang)?;
    let (config, pool) = load_config_and_pool().await?;

    let api_key = config.unsplash_api_key.as_ref().map(|k| k.expose());
    let count = wisecrow::media::prefetch::prefetch_media(
        &pool,
        &args.native_lang,
        &args.foreign_lang,
        args.audio,
        args.images,
        api_key,
    )
    .await?;

    info!("Prefetched {count} media items");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt::init();
    if let Err(e) = dotenvy::dotenv() {
        tracing::debug!("No .env file loaded: {e}");
    }
    let cli = Cli::parse();

    match cli.command {
        Command::Download(args) => handle_download(args).await?,
        Command::DownloadAll(args) => handle_download_all(args).await?,
        Command::GenerateExercises(args) => handle_generate_exercises(args).await?,
        Command::ImportGrammar(args) => handle_import_grammar(args).await?,
        Command::ImportPdf(args) => handle_import_pdf(args).await?,
        Command::Ingest(args) => handle_ingest(args).await?,
        Command::Learn(args) => handle_learn(args).await?,
        Command::Nback(args) => handle_nback(args).await?,
        Command::ListLanguages => {
            println!("{:<10} Language", "Code");
            println!("{}", "-".repeat(40));
            for (code, name) in SUPPORTED_LANGUAGE_INFO {
                println!("{code:<10} {name}");
            }
        }
        Command::PrefetchMedia(args) => handle_prefetch_media(args).await?,
        Command::Quiz(args) => handle_quiz(args)?,
        Command::SeedGrammar(args) => handle_seed_grammar(args).await?,
        Command::Sync(args) => handle_sync(args).await?,
    }
    Ok(())
}

async fn handle_nback(args: NbackArgs) -> Result<(), Error> {
    validate_languages(&args.native_lang, &args.foreign_lang)?;
    let (_config, pool) = load_config_and_pool().await?;

    let mode: wisecrow::dnb::DnbMode = args.mode.parse()?;
    let config = wisecrow::dnb::DnbConfig {
        mode,
        n_level: args.n_level,
        interval_ms: 4000,
    };

    let vocab = wisecrow::dnb::session::DnbSessionRepository::load_vocab(
        &pool,
        &args.native_lang,
        &args.foreign_lang,
        100,
    )
    .await?;

    if vocab.len() < 8 {
        info!(
            "Not enough vocabulary ({} items, need 8+). Ingest data first.",
            vocab.len()
        );
        return Ok(());
    }

    let session_id = wisecrow::dnb::session::DnbSessionRepository::create_session(
        &pool,
        args.user_id,
        &args.native_lang,
        &args.foreign_lang,
        mode,
        &wisecrow::dnb::scoring::AdaptationState::new(args.n_level, 4000),
    )
    .await?;

    let seed = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            % u128::from(u64::MAX),
    )
    .unwrap_or(42);

    let mut engine = wisecrow::dnb::DnbEngine::new(vocab, &config, seed)?;

    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::MoveTo(0, 0)
    )?;

    print_line(
        &mut stdout,
        &format!(
            "Dual N-Back ({mode}) | N={} | [A]=audio match  [L]=visual match  [Enter]=submit  [Q]=quit\r\n",
            args.n_level
        ),
    )?;
    print_line(&mut stdout, "\r\n")?;

    let mut trial_count = 0u32;
    let result = nback_game_loop(
        &mut engine,
        &pool,
        session_id,
        &mut stdout,
        &mut trial_count,
    )
    .await;

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(stdout, crossterm::cursor::Show)?;
    println!();

    if let Err(e) = result {
        error!("Game loop error: {e}");
    }

    let state = engine.state();
    let audio_acc = wisecrow::dnb::scoring::channel_accuracy(
        engine.completed_trials(),
        wisecrow::dnb::scoring::Channel::Audio,
        engine.completed_trials().len(),
    );
    let visual_acc = wisecrow::dnb::scoring::channel_accuracy(
        engine.completed_trials(),
        wisecrow::dnb::scoring::Channel::Visual,
        engine.completed_trials().len(),
    );

    wisecrow::dnb::session::DnbSessionRepository::complete_session(
        &pool,
        session_id,
        state,
        trial_count,
        #[expect(clippy::cast_possible_truncation)]
        Some(audio_acc as f32),
        #[expect(clippy::cast_possible_truncation)]
        Some(visual_acc as f32),
    )
    .await?;

    wisecrow::dnb::feedback::apply_srs_feedback(&pool, engine.completed_trials()).await?;

    println!(
        "\r\nSession complete: {} trials, N peak={}, audio={:.0}%, visual={:.0}%",
        trial_count,
        state.n_level_peak,
        audio_acc * 100.0,
        visual_acc * 100.0,
    );

    Ok(())
}

fn print_line(stdout: &mut std::io::Stdout, text: &str) -> Result<(), std::io::Error> {
    use std::io::Write;
    write!(stdout, "{text}")?;
    stdout.flush()
}

async fn nback_game_loop(
    engine: &mut wisecrow::dnb::DnbEngine,
    pool: &PgPool,
    session_id: i32,
    stdout: &mut std::io::Stdout,
    trial_count: &mut u32,
) -> Result<(), Error> {
    use std::time::{Duration, Instant};

    const MAX_TRIALS: u32 = 50;

    loop {
        let trial = engine.next_trial();
        *trial_count = trial_count.saturating_add(1);

        print_line(
            stdout,
            &format!(
                "\r\n--- Trial {} (N={}) ---\r\n",
                trial.trial_number, trial.n_level
            ),
        )?;
        print_line(
            stdout,
            &format!("  Audio:  {}\r\n", trial.audio_vocab.to_phrase),
        )?;
        print_line(
            stdout,
            &format!("  Visual: {}\r\n", trial.visual_vocab.from_phrase),
        )?;
        print_line(
            stdout,
            "  [A] audio match  [L] visual match  [Enter] submit\r\n",
        )?;

        let mut audio_pressed = false;
        let mut visual_pressed = false;
        let start = Instant::now();
        let deadline = Duration::from_millis(u64::from(trial.interval_ms));

        loop {
            let remaining = deadline.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                print_line(stdout, "  Time up!\r\n")?;
                break;
            }

            if crossterm::event::poll(remaining.min(Duration::from_millis(50)))? {
                if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                    if key.kind != crossterm::event::KeyEventKind::Press {
                        continue;
                    }
                    match key.code {
                        crossterm::event::KeyCode::Char('a')
                        | crossterm::event::KeyCode::Char('A') => {
                            audio_pressed = !audio_pressed;
                            let marker = if audio_pressed { "ON" } else { "off" };
                            print_line(stdout, &format!("  Audio match: {marker}\r\n"))?;
                        }
                        crossterm::event::KeyCode::Char('l')
                        | crossterm::event::KeyCode::Char('L') => {
                            visual_pressed = !visual_pressed;
                            let marker = if visual_pressed { "ON" } else { "off" };
                            print_line(stdout, &format!("  Visual match: {marker}\r\n"))?;
                        }
                        crossterm::event::KeyCode::Enter => break,
                        crossterm::event::KeyCode::Char('q')
                        | crossterm::event::KeyCode::Char('Q') => {
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            }
        }

        let elapsed_ms = u32::try_from(start.elapsed().as_millis()).unwrap_or(u32::MAX);
        let response = wisecrow::dnb::TrialResponse {
            audio_response: Some(audio_pressed),
            visual_response: Some(visual_pressed),
            response_time_ms: Some(elapsed_ms),
        };

        engine.record_response(response);

        if let Some(last) = engine.completed_trials().last() {
            let a_ok = if last.audio_correct() {
                "correct"
            } else {
                "wrong"
            };
            let v_ok = if last.visual_correct() {
                "correct"
            } else {
                "wrong"
            };
            print_line(
                stdout,
                &format!("  Result: audio={a_ok}, visual={v_ok}\r\n"),
            )?;

            wisecrow::dnb::session::DnbSessionRepository::save_trial(pool, session_id, last)
                .await?;
        }

        if engine.should_terminate() || *trial_count >= MAX_TRIALS {
            break;
        }
    }

    Ok(())
}

async fn handle_learn(args: LearnArgs) -> Result<(), Error> {
    validate_languages(&args.native_lang, &args.foreign_lang)?;
    let (config, pool) = load_config_and_pool().await?;

    let session =
        match SessionManager::resume(&pool, args.user_id, &args.native_lang, &args.foreign_lang)
            .await?
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
                    args.user_id,
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
    let path = std::path::Path::new(&args.pdf_path)
        .canonicalize()
        .map_err(|_| {
            WisecrowError::InvalidInput(format!("PDF file not found: {}", args.pdf_path))
        })?;
    quiz::run_quiz(&path, args.num_questions)?;
    Ok(())
}
