//! The reader job against PostgreSQL: book storage, batch atomicity, and
//! (from later tasks) each step of `prepare-readers`.

use sqlx::PgPool;
use wisecrow::reader::import::{
    advance, lock_progress, save_run, store_book, BookStatus, ImportIdentity, NewBook, AUTO_PAGES,
};
use wisecrow::reader::segment::SegmentedSentence;
use wisecrow::reader::shelf::ReaderEntry;

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn reset_pool() -> Result<PgPool, Box<dyn std::error::Error>> {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    sqlx::query(
        "TRUNCATE books, book_glosses, reader_translations, reading_positions, decks CASCADE",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('es', 'Spanish')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(&pool)
    .await?;
    Ok(pool)
}

fn entry(file: &str) -> ReaderEntry {
    ReaderEntry {
        series: "Penguin Readers".into(),
        level: "2".into(),
        title: "The Lighthouse Keeper".into(),
        file: file.into(),
        cefr: Ok("A2"),
        pages: Ok(None),
        translate: true,
    }
}

fn sentence(paragraph: u32, sentence: u32, text: &str) -> SegmentedSentence {
    SegmentedSentence {
        chapter: 1,
        paragraph,
        sentence,
        text: text.into(),
    }
}

fn three_sentences() -> Vec<SegmentedSentence> {
    vec![
        sentence(1, 1, "Tom lived near the sea."),
        sentence(1, 2, "It was a quiet place."),
        sentence(2, 1, "‘No,’ said Tom."),
    ]
}

async fn language(pool: &PgPool, code: &str) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM languages WHERE code = $1")
        .bind(code)
        .fetch_one(pool)
        .await
}

async fn stored(pool: &PgPool, pages: &str) -> Result<i32, Box<dyn std::error::Error>> {
    let entry = entry("Penguin Readers/Level 2/Keeper.pdf");
    let sentences = three_sentences();
    let id = store_book(
        pool,
        &NewBook {
            entry: &entry,
            identity: ImportIdentity {
                file_sha256: [3; 32],
                pages,
                segmenter_version: 1,
            },
            cefr_level: "A2",
            status: BookStatus::Ready,
            pages: Some((6, 33)),
            sentences: &sentences,
        },
    )
    .await?
    .ok_or("the first store creates the book")?;
    Ok(id)
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_book_is_stored_once_with_its_ordered_sentences() -> TestResult {
    let pool = reset_pool().await?;
    let book = stored(&pool, AUTO_PAGES).await?;
    let entry = entry("Penguin Readers/Level 2/Keeper.pdf");
    let sentences = three_sentences();

    let again = store_book(
        &pool,
        &NewBook {
            entry: &entry,
            identity: ImportIdentity {
                file_sha256: [3; 32],
                pages: AUTO_PAGES,
                segmenter_version: 1,
            },
            cefr_level: "A2",
            status: BookStatus::Ready,
            pages: Some((6, 33)),
            sentences: &sentences,
        },
    )
    .await?;

    assert_eq!(again, None, "the same identity is not stored twice");
    let rows: Vec<(i32, i32, i32, i32, String)> = sqlx::query_as(
        "SELECT position, chapter, paragraph, sentence, text FROM book_sentences
         WHERE book_id = $1 ORDER BY position",
    )
    .bind(book)
    .fetch_all(&pool)
    .await?;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[2], (2, 1, 2, 1, "‘No,’ said Tom.".to_owned()));
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_changed_page_range_is_a_new_revision_of_the_same_file() -> TestResult {
    let pool = reset_pool().await?;
    let first = stored(&pool, AUTO_PAGES).await?;
    let second = stored(&pool, "6-33").await?;

    assert_ne!(first, second);
    let files: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT file FROM books WHERE id IN ($1, $2)")
            .bind(first)
            .bind(second)
            .fetch_all(&pool)
            .await?;
    assert_eq!(files, ["Penguin Readers/Level 2/Keeper.pdf"]);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_interrupted_batch_leaves_cursor_and_content_as_they_were() -> TestResult {
    let pool = reset_pool().await?;
    let book = stored(&pool, AUTO_PAGES).await?;
    let spanish = language(&pool, "es").await?;
    let ids: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM book_sentences WHERE book_id = $1 ORDER BY position")
            .bind(book)
            .fetch_all(&pool)
            .await?;
    let run = vec![
        (ids[0], "Tom vivía cerca del mar.".to_owned()),
        (ids[1], "Era un lugar tranquilo.".to_owned()),
    ];

    let mut interrupted = pool.begin().await?;
    lock_progress(&mut interrupted, book, spanish).await?;
    save_run(&mut interrupted, spanish, &run).await?;
    advance(&mut interrupted, book, spanish, 2, None, 3).await?;
    interrupted.rollback().await?;

    let translated: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM book_translations")
        .fetch_one(&pool)
        .await?;
    assert_eq!(translated, 0, "no translation survives the rollback");
    let mut fresh = pool.begin().await?;
    let progress = lock_progress(&mut fresh, book, spanish).await?;
    assert_eq!(progress.cursor, 0, "nor does the cursor");
    save_run(&mut fresh, spanish, &run).await?;
    advance(&mut fresh, book, spanish, 2, None, 3).await?;
    fresh.commit().await?;

    let tokens: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM book_tokens k JOIN book_translations t ON t.id = k.translation_id
         WHERE t.sentence_id = $1",
    )
    .bind(ids[0])
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        tokens, 5,
        "Tom vivía cerca del mar: five words stored with the sentence"
    );
    let (cursor, stage): (i32, String) = sqlx::query_as(
        "SELECT cursor, stage FROM book_import_progress WHERE book_id = $1 AND language_id = $2",
    )
    .bind(book)
    .bind(spanish)
    .fetch_one(&pool)
    .await?;
    assert_eq!((cursor, stage.as_str()), (2, "translate"));
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_failed_run_is_recorded_and_passed() -> TestResult {
    let pool = reset_pool().await?;
    let book = stored(&pool, AUTO_PAGES).await?;
    let spanish = language(&pool, "es").await?;

    let mut transaction = pool.begin().await?;
    lock_progress(&mut transaction, book, spanish).await?;
    advance(&mut transaction, book, spanish, 3, Some(0), 3).await?;
    transaction.commit().await?;

    let (cursor, failed, stage, completed): (i32, Vec<i32>, String, bool) = sqlx::query_as(
        "SELECT cursor, failed_runs, stage, completed_at IS NOT NULL FROM book_import_progress
         WHERE book_id = $1 AND language_id = $2",
    )
    .bind(book)
    .bind(spanish)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        (cursor, failed, stage.as_str(), completed),
        (3, vec![0], "complete", true)
    );
    Ok(())
}
use std::collections::VecDeque;
use std::sync::Mutex;
use wisecrow::llm::cache::CachedLlm;
use wisecrow::llm::LlmProvider;
use wisecrow::reader::import::{translate_book_run, RunStored};

struct Scripted {
    answers: Mutex<VecDeque<Result<String, wisecrow::errors::WisecrowError>>>,
}

#[async_trait::async_trait]
impl LlmProvider for Scripted {
    async fn generate(&self, _: &str, _: u32) -> Result<String, wisecrow::errors::WisecrowError> {
        self.answers
            .lock()
            .expect("script")
            .pop_front()
            .unwrap_or_else(|| {
                Err(wisecrow::errors::WisecrowError::LlmError(
                    "script exhausted".into(),
                ))
            })
    }
    fn name(&self) -> &str {
        "scripted"
    }
}

fn numbered(texts: &[&str]) -> Result<String, wisecrow::errors::WisecrowError> {
    let entries: Vec<serde_json::Value> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| serde_json::json!({ "n": index + 1, "text": text }))
        .collect();
    Ok(serde_json::json!({ "translations": entries }).to_string())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_translation_run_is_stored_with_its_tokens_in_one_commit() -> TestResult {
    let pool = reset_pool().await?;
    let book = stored(&pool, AUTO_PAGES).await?;
    let provider = Scripted {
        answers: Mutex::new(VecDeque::from([numbered(&["Uno.", "Dos.", "Tres."])])),
    };
    let model = CachedLlm::new(&provider, None);

    let stored_run = translate_book_run(&pool, book, "es", "Spanish", "A2", &model).await?;

    assert_eq!(stored_run, RunStored::Advanced { cursor: 3 });
    let tokens: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM book_tokens")
        .fetch_one(&pool)
        .await?;
    assert!(tokens >= 3, "each stored sentence has its spans");
    let again = translate_book_run(&pool, book, "es", "Spanish", "A2", &model).await?;
    assert_eq!(again, RunStored::Done);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_run_rejected_twice_is_recorded_and_the_cursor_moves() -> TestResult {
    let pool = reset_pool().await?;
    let book = stored(&pool, AUTO_PAGES).await?;
    let provider = Scripted {
        answers: Mutex::new(VecDeque::from([
            Ok("not json".into()),
            Ok("still not json".into()),
        ])),
    };
    let model = CachedLlm::new(&provider, None);

    let stored_run = translate_book_run(&pool, book, "es", "Spanish", "A2", &model).await?;

    assert_eq!(
        stored_run,
        RunStored::Rejected {
            cursor: 3,
            start: 0
        }
    );
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM book_translations")
        .fetch_one(&pool)
        .await?;
    assert_eq!(rows, 0, "a rejected run stores no sentence");
    let failed: Vec<i32> =
        sqlx::query_scalar("SELECT failed_runs FROM book_import_progress WHERE book_id = $1")
            .bind(book)
            .fetch_one(&pool)
            .await?;
    assert_eq!(failed, vec![0]);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn an_outage_leaves_the_cursor_where_it_was() -> TestResult {
    let pool = reset_pool().await?;
    let book = stored(&pool, AUTO_PAGES).await?;
    let provider = Scripted {
        answers: Mutex::new(VecDeque::from([Err(
            wisecrow::errors::WisecrowError::LlmUnavailable("limit".into()),
        )])),
    };
    let model = CachedLlm::new(&provider, None);

    let error = translate_book_run(&pool, book, "es", "Spanish", "A2", &model)
        .await
        .expect_err("outage");

    assert!(matches!(
        error,
        wisecrow::errors::WisecrowError::LlmUnavailable(_)
    ));
    let progress: Option<(i32, Vec<i32>)> =
        sqlx::query_as("SELECT cursor, failed_runs FROM book_import_progress WHERE book_id = $1")
            .bind(book)
            .fetch_optional(&pool)
            .await?;
    assert!(
        progress.is_none() || progress == Some((0, vec![])),
        "the cursor did not move"
    );
    Ok(())
}
use wisecrow::reader::notes::{gloss_forms, GlossStored};

fn note_json(form: &str) -> String {
    serde_json::json!({
        "notes": [{
            "form": form,
            "meaning": "lived",
            "lemma": "vivir",
            "class": "verb",
            "note": "Third person, past."
        }]
    })
    .to_string()
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_gloss_batch_stores_a_note_and_a_failure() -> TestResult {
    let pool = reset_pool().await?;
    let book = stored(&pool, AUTO_PAGES).await?;
    let spanish = language(&pool, "es").await?;
    let mut transaction = pool.begin().await?;
    lock_progress(&mut transaction, book, spanish).await?;
    let sentence_id: i32 = sqlx::query_scalar(
        "SELECT id FROM book_sentences WHERE book_id = $1 ORDER BY position LIMIT 1",
    )
    .bind(book)
    .fetch_one(&mut *transaction)
    .await?;
    save_run(
        &mut transaction,
        spanish,
        &[(sentence_id, "Tom vivía.".into())],
    )
    .await?;
    transaction.commit().await?;
    let provider = Scripted {
        answers: Mutex::new(VecDeque::from([
            Ok(note_json("tom")),
            Ok(r#"{"notes":[]}"#.into()),
        ])),
    };
    let model = CachedLlm::new(&provider, Some(&pool));

    let first = gloss_forms(&pool, "es", "en", &model).await?;
    let second = gloss_forms(&pool, "es", "en", &model).await?;

    assert!(matches!(first, GlossStored::Stored { ok: 1, .. }));
    assert_eq!(
        second,
        GlossStored::Done,
        "a stored failure is not asked again"
    );
    let failed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM book_glosses WHERE status = 'failed'")
            .fetch_one(&pool)
            .await?;
    assert!(
        failed >= 1,
        "the forms the answer omitted are stored as failed"
    );
    Ok(())
}

use std::sync::atomic::{AtomicUsize, Ordering};
use wisecrow::media::fingerprint::MediaFingerprint;
use wisecrow::media::grammar_audio::SentenceSpeaker;

struct Stub;

impl Stub {
    fn calls() -> usize {
        STUB_CALLS.load(Ordering::SeqCst)
    }
}

static STUB_CALLS: AtomicUsize = AtomicUsize::new(0);

#[async_trait::async_trait]
impl SentenceSpeaker for Stub {
    fn fingerprint(
        &self,
        language: &str,
        sentence: &str,
    ) -> Result<MediaFingerprint, wisecrow::errors::WisecrowError> {
        Ok(MediaFingerprint::for_audio(
            sentence, language, "stub", "stub", 1,
        ))
    }

    async fn speak(
        &self,
        _language: &str,
        _sentence: &str,
    ) -> Result<Vec<u8>, wisecrow::errors::WisecrowError> {
        STUB_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(b"clip".to_vec())
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn voicing_a_sentence_stores_its_fingerprint_once() -> TestResult {
    STUB_CALLS.store(0, Ordering::SeqCst);
    let pool = reset_pool().await?;
    let book = stored(&pool, AUTO_PAGES).await?;
    let spanish = language(&pool, "es").await?;
    let mut transaction = pool.begin().await?;
    lock_progress(&mut transaction, book, spanish).await?;
    let sentence_id: i32 = sqlx::query_scalar(
        "SELECT id FROM book_sentences WHERE book_id = $1 ORDER BY position LIMIT 1",
    )
    .bind(book)
    .fetch_one(&mut *transaction)
    .await?;
    save_run(&mut transaction, spanish, &[(sentence_id, "Hola.".into())]).await?;
    transaction.commit().await?;

    let cache = std::env::temp_dir();
    let voiced =
        wisecrow::reader::audio::voice_sentence_batch(&pool, "es", &Stub, &cache, 100).await?;
    let again =
        wisecrow::reader::audio::voice_sentence_batch(&pool, "es", &Stub, &cache, 100).await?;

    assert_eq!(voiced, 1);
    assert_eq!(again, 0, "a stored fingerprint is not spoken again");
    assert_eq!(Stub::calls(), 1, "one synthesis");
    Ok(())
}

fn reader_pairs() -> Vec<wisecrow::pipeline::PairSpec> {
    vec![wisecrow::pipeline::PairSpec {
        native: "en".into(),
        foreign: "es".into(),
        corpora: None,
    }]
}

fn gloss_notes(forms: &[(&str, &str)]) -> String {
    let notes: Vec<serde_json::Value> = forms
        .iter()
        .map(|(form, meaning)| {
            serde_json::json!({
                "form": form,
                "meaning": meaning,
                "lemma": form,
                "class": "noun",
                "note": "A short note."
            })
        })
        .collect();
    serde_json::json!({ "notes": notes }).to_string()
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn prepare_reader_step_translates_then_idles_and_skips_an_uncleared_book() -> TestResult {
    let pool = reset_pool().await?;
    let shelf = tempfile::tempdir()?;
    std::fs::write(
        shelf.path().join("SOURCES.md"),
        "\
| Series | Level | Title | File | CEFR | Pages | Translate | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Penguin Readers | 2 | Secret | `secret.pdf` | A2 |  |  |  |
",
    )?;
    let provider = Scripted {
        answers: Mutex::new(VecDeque::new()),
    };
    let model = CachedLlm::new(&provider, None);

    let skipped =
        wisecrow::pipeline::prepare_reader_step(&pool, shelf.path(), &reader_pairs(), &model, None)
            .await?;

    assert_eq!(skipped, wisecrow::pipeline::CorpusStep::Idle);
    assert_eq!(provider.answers.lock().expect("script").len(), 0);
    let translations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM book_translations")
        .fetch_one(&pool)
        .await?;
    assert_eq!(translations, 0, "a blank Translate cell is not sent");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn prepare_reader_step_walks_a_stored_book_and_is_idle_on_the_second_pass() -> TestResult {
    let pool = reset_pool().await?;
    stored(&pool, AUTO_PAGES).await?;
    let shelf = tempfile::tempdir()?;
    std::fs::write(
        shelf.path().join("SOURCES.md"),
        "\
| Series | Level | Title | File | CEFR | Pages | Translate | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- |
",
    )?;
    let provider = Scripted {
        answers: Mutex::new(VecDeque::from([
            numbered(&["Uno.", "Dos.", "Tres."]),
            Ok(gloss_notes(&[
                ("dos", "two"),
                ("tres", "three"),
                ("uno", "one"),
            ])),
            Ok("unused".into()),
        ])),
    };
    let model = CachedLlm::new(&provider, None);
    let mut saw_translation = false;
    let mut idle_prompts = None;

    for _ in 0..6 {
        let step = wisecrow::pipeline::prepare_reader_step(
            &pool,
            shelf.path(),
            &reader_pairs(),
            &model,
            None,
        )
        .await?;
        if !saw_translation {
            assert_eq!(step, wisecrow::pipeline::CorpusStep::Worked);
            let translations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM book_translations")
                .fetch_one(&pool)
                .await?;
            assert_eq!(translations, 3);
            saw_translation = true;
            continue;
        }
        if step == wisecrow::pipeline::CorpusStep::Idle {
            let prompts = provider.answers.lock().expect("script").len();
            match idle_prompts {
                None => idle_prompts = Some(prompts),
                Some(earlier) => {
                    assert_eq!(prompts, earlier, "idle does not call the model again");
                    break;
                }
            }
        } else {
            assert_eq!(step, wisecrow::pipeline::CorpusStep::Worked);
        }
    }

    assert!(saw_translation);
    assert!(idle_prompts.is_some(), "the finished book goes idle");
    let voiced: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM book_translations WHERE audio_fingerprint IS NOT NULL",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(voiced, 0, "no speaker means a null fingerprint is not work");
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn a_chapter_is_readable_only_when_every_sentence_is_translated_and_none_failed() -> TestResult
{
    use wisecrow::reader::import::save_run;
    use wisecrow::reader::library::{library_books, save_reading_position};

    let pool = reset_pool().await?;
    let spanish = language(&pool, "es").await?;
    let user: i32 = sqlx::query_scalar(
        "INSERT INTO users (display_name, email) VALUES ('Reader', 'library@localhost.test')
         ON CONFLICT (email) DO UPDATE SET display_name = EXCLUDED.display_name
         RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    let first = stored(&pool, AUTO_PAGES).await?;
    let ids: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM book_sentences WHERE book_id = $1 ORDER BY position")
            .bind(first)
            .fetch_all(&pool)
            .await?;
    let mut transaction = pool.begin().await?;
    save_run(
        &mut transaction,
        spanish,
        &[
            (ids[0], "Tom vivía.".into()),
            (ids[1], "Era un lugar.".into()),
        ],
    )
    .await?;
    transaction.commit().await?;

    let partial = library_books(&pool, user, "es").await?;
    let partial = partial.iter().find(|book| book.id == first).expect("book");
    assert!(!partial.readable);

    let mut transaction = pool.begin().await?;
    save_run(
        &mut transaction,
        spanish,
        &[(ids[2], "No, dijo Tom.".into())],
    )
    .await?;
    transaction.commit().await?;
    let ready = library_books(&pool, user, "es").await?;
    let ready = ready.iter().find(|book| book.id == first).expect("book");
    assert!(ready.readable);

    let second = stored(&pool, "6-33").await?;
    let second_ids: Vec<i32> =
        sqlx::query_scalar("SELECT id FROM book_sentences WHERE book_id = $1 ORDER BY position")
            .bind(second)
            .fetch_all(&pool)
            .await?;
    let mut transaction = pool.begin().await?;
    save_run(
        &mut transaction,
        spanish,
        &[
            (second_ids[0], "Tom vivía.".into()),
            (second_ids[1], "Era un lugar.".into()),
            (second_ids[2], "No, dijo Tom.".into()),
        ],
    )
    .await?;
    transaction.commit().await?;
    sqlx::query(
        "INSERT INTO book_import_progress (book_id, language_id, failed_runs)
         VALUES ($1, $2, ARRAY[0]::int[])
         ON CONFLICT (book_id, language_id)
         DO UPDATE SET failed_runs = EXCLUDED.failed_runs",
    )
    .bind(second)
    .bind(spanish)
    .execute(&pool)
    .await?;
    let failed = library_books(&pool, user, "es").await?;
    let failed = failed.iter().find(|book| book.id == second).expect("book");
    assert!(!failed.readable, "a failed run keeps the book closed");

    save_reading_position(&pool, user, first, 2, 1, 1).await?;
    let placed = library_books(&pool, user, "es").await?;
    let placed = placed.iter().find(|book| book.id == first).expect("book");
    assert_eq!(
        (placed.chapter, placed.paragraph, placed.sentence),
        (Some(2), Some(1), Some(1))
    );
    Ok(())
}
