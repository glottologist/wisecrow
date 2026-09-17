//! Mapped ingestion: an exact archive tag (`zh_CN`) is persisted under the
//! canonical application code (`zh`), and every failure mode of an import —
//! nothing accepted, malformed XML, database rejection — reaches the caller
//! as an error rather than a success log line.

mod common;

use std::io::Write;
use tempfile::NamedTempFile;
use wisecrow::downloader::DownloadConfig;
use wisecrow::errors::WisecrowError;
use wisecrow::files::{IngestLanguage, IngestLanguages};
use wisecrow::ingesting::Ingester;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn tmx_file(content: &str) -> Result<NamedTempFile, Box<dyn std::error::Error>> {
    let mut file = NamedTempFile::with_suffix(".tmx")?;
    file.write_all(content.as_bytes())?;
    Ok(file)
}

fn path_of(file: &NamedTempFile) -> Result<&str, Box<dyn std::error::Error>> {
    Ok(file.path().to_str().ok_or("UTF-8 path required")?)
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn alias_persists_canonical_pair_and_zero_is_error() -> TestResult {
    let pool = common::test_pool().await;
    common::truncate_tables(&pool).await;
    let file = tmx_file(&common::generate_tmx(
        &[("I learn", "我学习")],
        "en",
        "zh_CN",
    ))?;
    let path = path_of(&file)?;
    let languages = IngestLanguages::new(
        IngestLanguage::new("en", "en")?,
        IngestLanguage::new("zh", "zh_CN")?,
    )?;
    let ingester = Ingester::new(pool.clone(), DownloadConfig::default());

    // Identity tags do not match zh_CN, so nothing is accepted: that is an
    // error, not a zero-row success.
    let mismatch = ingester
        .ingest_from_file(path, "identity mismatch", "en", "zh")
        .await;
    assert!(
        matches!(mismatch, Err(WisecrowError::InvalidInput(_))),
        "expected InvalidInput, got {mismatch:?}"
    );
    assert_eq!(common::count_translations(&pool).await, 0);

    ingester
        .ingest_from_file_with_languages(path, "Chinese fixture", &languages)
        .await?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT fl.code, tl.code FROM translations t \
         JOIN languages fl ON fl.id = t.from_language_id \
         JOIN languages tl ON tl.id = t.to_language_id",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(rows, [(String::from("en"), String::from("zh"))]);
    assert_eq!(
        common::get_translation_pairs(&pool).await,
        [(String::from("I learn"), String::from("我学习"))]
    );
    // The source tag never becomes a language record.
    assert_eq!(common::count_languages(&pool).await, 2);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn malformed_xml_fails_the_import() -> TestResult {
    let pool = common::test_pool().await;
    common::truncate_tables(&pool).await;
    let file = tmx_file(
        "<tmx><body><tu><tuv xml:lang=\"en\"><seg>Hello</seg></tuv>\
         <tuv xml:lang=\"es\"><seg>Hola</seg></tuv></body></tmx>",
    )?;
    let ingester = Ingester::new(pool.clone(), DownloadConfig::default());
    let result = ingester
        .ingest_from_file(path_of(&file)?, "malformed", "en", "es")
        .await;
    assert!(
        matches!(result, Err(WisecrowError::InvalidInput(_))),
        "expected InvalidInput, got {result:?}"
    );
    assert_eq!(common::count_translations(&pool).await, 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn database_rejection_fails_the_import() -> TestResult {
    let pool = common::test_pool().await;
    common::truncate_tables(&pool).await;
    // A fixture-only constraint stands in for any persistence failure; it is
    // removed before the assertions so a failing run cannot poison later tests.
    sqlx::query(
        "ALTER TABLE translations ADD CONSTRAINT fixture_reject CHECK (to_phrase <> 'rejected')",
    )
    .execute(&pool)
    .await?;
    let file = tmx_file(&common::generate_tmx(&[("Hello", "rejected")], "en", "es"))?;
    let ingester = Ingester::new(pool.clone(), DownloadConfig::default());
    let result = ingester
        .ingest_from_file(path_of(&file)?, "rejected", "en", "es")
        .await;
    sqlx::query("ALTER TABLE translations DROP CONSTRAINT fixture_reject")
        .execute(&pool)
        .await?;
    assert!(
        matches!(result, Err(WisecrowError::PersistenceConnectionError(_))),
        "expected PersistenceConnectionError, got {result:?}"
    );
    assert_eq!(common::count_translations(&pool).await, 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn identity_xml_alignment_import_survives_channel_backpressure() -> TestResult {
    // The parser and persister are joined in one call over a bounded channel;
    // more pairs than the channel holds must still flow through without a
    // deadlock, and the alignment format keeps its canonical-code path.
    let pool = common::test_pool().await;
    common::truncate_tables(&pool).await;
    let owned = common::make_pairs(2500);
    let pairs: Vec<(&str, &str)> = owned
        .iter()
        .map(|(source, target)| (source.as_str(), target.as_str()))
        .collect();
    let mut file = NamedTempFile::with_suffix(".xml")?;
    file.write_all(common::generate_xml_alignment(&pairs, "en", "es").as_bytes())?;
    let ingester = Ingester::new(pool.clone(), DownloadConfig::default());
    ingester
        .ingest_from_file(path_of(&file)?, "alignment", "en", "es")
        .await?;
    assert_eq!(common::count_translations(&pool).await, 2500);
    Ok(())
}
