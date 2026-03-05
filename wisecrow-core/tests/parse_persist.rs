mod common;

use rstest::rstest;
use std::io::Write;
use tempfile::NamedTempFile;
use tokio::sync::mpsc;
use wisecrow::ingesting::parsing::{CorpusParser, TranslationPair};
use wisecrow::ingesting::persisting::DatabasePersister;

/// Writes TMX content to a temp file and returns it.
fn tmx_temp_file(pairs: &[(&str, &str)], src: &str, tgt: &str) -> NamedTempFile {
    let content = common::generate_tmx(pairs, src, tgt);
    let mut tmp = NamedTempFile::new().expect("Failed to create temp file");
    tmp.write_all(content.as_bytes())
        .expect("Failed to write temp file");
    tmp
}

/// Writes XML alignment content to a temp file and returns it.
fn xml_temp_file(pairs: &[(&str, &str)], src: &str, tgt: &str) -> NamedTempFile {
    let content = common::generate_xml_alignment(pairs, src, tgt);
    let mut tmp = NamedTempFile::new().expect("Failed to create temp file");
    tmp.write_all(content.as_bytes())
        .expect("Failed to write temp file");
    tmp
}

/// Runs the full parse→channel→persist flow for a TMX temp file.
async fn parse_and_persist_tmx(pool: &sqlx::PgPool, pairs: &[(&str, &str)], src: &str, tgt: &str) {
    let tmp = tmx_temp_file(pairs, src, tgt);
    let (tx, rx) = mpsc::channel::<TranslationPair>(1000);

    let persister = DatabasePersister::new(pool.clone()); // clone: PgPool is Arc-based
    let from_id = persister.ensure_language(src, src).await.unwrap();
    let to_id = persister.ensure_language(tgt, tgt).await.unwrap();

    let path = tmp.path().to_str().unwrap().to_owned();
    let src_owned = src.to_owned();
    let tgt_owned = tgt.to_owned();

    let parse_handle = tokio::spawn(async move {
        CorpusParser::parse_tmx_file(&path, &src_owned, &tgt_owned, &tx)
            .await
            .unwrap()
    });

    let persist_handle =
        tokio::spawn(async move { persister.consume(rx, from_id, to_id).await.unwrap() });

    let (count, ()) = tokio::try_join!(parse_handle, persist_handle).unwrap();
    assert_eq!(count, pairs.len());
}

#[rstest]
#[case(999)]
#[case(1000)]
#[case(1001)]
#[case(2500)]
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn batch_boundaries(#[case] n: usize) {
    let pool = common::test_pool().await;
    common::truncate_tables(&pool).await;

    let owned_pairs = common::make_pairs(n);
    let pair_refs: Vec<(&str, &str)> = owned_pairs
        .iter()
        .map(|(s, t)| (s.as_str(), t.as_str()))
        .collect();

    parse_and_persist_tmx(&pool, &pair_refs, "en", "es").await;

    let count = common::count_translations(&pool).await;
    assert_eq!(count, i64::try_from(n).unwrap());
}

#[rstest]
#[case("tmx")]
#[case("xml")]
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn parse_persist_roundtrip_by_format(#[case] format: &str) {
    let pool = common::test_pool().await;
    common::truncate_tables(&pool).await;

    let pairs: Vec<(&str, &str)> = vec![("Hello", "Hola"), ("Goodbye", "Adiós")];

    let tmp = match format {
        "tmx" => tmx_temp_file(&pairs, "en", "es"),
        "xml" => xml_temp_file(&pairs, "en", "es"),
        other => panic!("Unsupported format: {other}"),
    };

    let (tx, rx) = mpsc::channel::<TranslationPair>(100);
    let persister = DatabasePersister::new(pool.clone()); // clone: PgPool is Arc-based
    let from_id = persister.ensure_language("en", "en").await.unwrap();
    let to_id = persister.ensure_language("es", "es").await.unwrap();

    let path = tmp.path().to_str().unwrap().to_owned();
    let is_tmx = format == "tmx";

    let parse_handle = tokio::spawn(async move {
        if is_tmx {
            CorpusParser::parse_tmx_file(&path, "en", "es", &tx).await
        } else {
            CorpusParser::parse_xml_alignment_file(&path, "en", "es", &tx).await
        }
        .unwrap()
    });

    let persist_handle =
        tokio::spawn(async move { persister.consume(rx, from_id, to_id).await.unwrap() });

    tokio::try_join!(parse_handle, persist_handle).unwrap();

    let count = common::count_translations(&pool).await;
    assert_eq!(count, 2);

    let db_pairs = common::get_translation_pairs(&pool).await;
    assert_eq!(db_pairs[0], ("Goodbye".to_owned(), "Adiós".to_owned()));
    assert_eq!(db_pairs[1], ("Hello".to_owned(), "Hola".to_owned()));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn duplicate_translations_ignored() {
    let pool = common::test_pool().await;
    common::truncate_tables(&pool).await;

    let pairs: Vec<(&str, &str)> = vec![("Hello", "Hola"), ("Goodbye", "Adiós")];

    parse_and_persist_tmx(&pool, &pairs, "en", "es").await;
    assert_eq!(common::count_translations(&pool).await, 2);

    parse_and_persist_tmx(&pool, &pairs, "en", "es").await;
    assert_eq!(common::count_translations(&pool).await, 2);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn empty_file_persists_nothing() {
    let pool = common::test_pool().await;
    common::truncate_tables(&pool).await;

    let empty_pairs: Vec<(&str, &str)> = vec![];
    parse_and_persist_tmx(&pool, &empty_pairs, "en", "es").await;

    assert_eq!(common::count_translations(&pool).await, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn ensure_language_idempotent() {
    let pool = common::test_pool().await;
    common::truncate_tables(&pool).await;

    let persister = DatabasePersister::new(pool.clone()); // clone: PgPool is Arc-based

    let id1 = persister.ensure_language("en", "English").await.unwrap();
    let id2 = persister.ensure_language("en", "English").await.unwrap();

    assert_eq!(id1, id2);
    assert_eq!(common::count_languages(&pool).await, 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn phrase_exceeding_1000_chars_rejected() {
    let pool = common::test_pool().await;
    common::truncate_tables(&pool).await;

    let long_phrase: String = "x".repeat(1001);
    let pairs: Vec<(&str, &str)> = vec![(long_phrase.as_str(), "short")];

    let tmp = tmx_temp_file(&pairs, "en", "es");
    let (tx, rx) = mpsc::channel::<TranslationPair>(100);

    let persister = DatabasePersister::new(pool.clone()); // clone: PgPool is Arc-based
    let from_id = persister.ensure_language("en", "en").await.unwrap();
    let to_id = persister.ensure_language("es", "es").await.unwrap();

    let path = tmp.path().to_str().unwrap().to_owned();

    let parse_handle = tokio::spawn(async move {
        CorpusParser::parse_tmx_file(&path, "en", "es", &tx)
            .await
            .unwrap()
    });

    let persist_handle = tokio::spawn(async move { persister.consume(rx, from_id, to_id).await });

    let (_count, persist_result) = tokio::try_join!(parse_handle, persist_handle).unwrap();

    assert!(
        persist_result.is_err(),
        "Expected CHECK constraint violation for phrase >1000 chars"
    );
}
