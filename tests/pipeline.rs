mod common;

use rstest::rstest;
use std::io::Write;
use tempfile::Builder;
use wisecrow::downloader::DownloadConfig;
use wisecrow::files::{Compression, Corpus, LanguageFileInfo};
use wisecrow::ingesting::Ingester;

/// Creates a temp file with the given extension and content.
fn temp_file_with_extension(extension: &str, content: &str) -> tempfile::NamedTempFile {
    let tmp = Builder::new()
        .suffix(&format!(".{extension}"))
        .tempfile()
        .expect("Failed to create temp file");
    tmp.as_file()
        .set_len(0)
        .expect("Failed to truncate temp file");
    let mut file = std::io::BufWriter::new(tmp.as_file());
    file.write_all(content.as_bytes())
        .expect("Failed to write temp file");
    file.flush().expect("Failed to flush temp file");
    drop(file);
    tmp
}

fn make_file_info(file_name: &str) -> LanguageFileInfo {
    LanguageFileInfo {
        corpus: Corpus::OpenSubtitles,
        target_location: String::new(),
        file_name: file_name.to_owned(),
        compressed: Compression::GzCompressed,
    }
}

#[rstest]
#[case("tmx", 10)]
#[case("tmx", 100)]
#[case("tmx", 1500)]
#[case("xml", 10)]
#[case("xml", 100)]
#[case("xml", 1500)]
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn ingest_local_file(#[case] format: &str, #[case] n: usize) {
    let pool = common::test_pool().await;
    common::truncate_tables(&pool).await;

    let owned_pairs = common::make_pairs(n);
    let pair_refs: Vec<(&str, &str)> = owned_pairs
        .iter()
        .map(|(s, t)| (s.as_str(), t.as_str()))
        .collect();

    let content = match format {
        "tmx" => common::generate_tmx(&pair_refs, "en", "es"),
        "xml" => common::generate_xml_alignment(&pair_refs, "en", "es"),
        other => panic!("Unsupported format: {other}"),
    };

    let tmp = temp_file_with_extension(format, &content);
    let path = tmp.path().to_str().unwrap().to_owned();
    let file_info = make_file_info(&path);

    let config = DownloadConfig::default();
    let ingester = Ingester::new(pool.clone(), config); // clone: PgPool is Arc-based

    ingester
        .ingest_from_file(&path, &file_info, "en", "es")
        .await
        .unwrap();

    let count = common::count_translations(&pool).await;
    assert_eq!(count, i64::try_from(n).unwrap());
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn ingest_same_file_twice_idempotent() {
    let pool = common::test_pool().await;
    common::truncate_tables(&pool).await;

    let pairs: Vec<(&str, &str)> = vec![("Hello", "Hola"), ("Goodbye", "Adiós")];
    let content = common::generate_tmx(&pairs, "en", "es");

    let tmp = temp_file_with_extension("tmx", &content);
    let path = tmp.path().to_str().unwrap().to_owned();
    let file_info = make_file_info(&path);

    let config = DownloadConfig::default();
    let ingester = Ingester::new(pool.clone(), config); // clone: PgPool is Arc-based

    ingester
        .ingest_from_file(&path, &file_info, "en", "es")
        .await
        .unwrap();
    assert_eq!(common::count_translations(&pool).await, 2);

    ingester
        .ingest_from_file(&path, &file_info, "en", "es")
        .await
        .unwrap();
    assert_eq!(common::count_translations(&pool).await, 2);

    let db_pairs = common::get_translation_pairs(&pool).await;
    assert_eq!(db_pairs.len(), 2);
    assert_eq!(db_pairs[0], ("Goodbye".to_owned(), "Adiós".to_owned()));
    assert_eq!(db_pairs[1], ("Hello".to_owned(), "Hola".to_owned()));
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn concurrent_ingest_different_languages() {
    let pool = common::test_pool().await;
    common::truncate_tables(&pool).await;

    let en_es_pairs: Vec<(&str, &str)> = vec![("Hello", "Hola"), ("World", "Mundo")];
    let en_fr_pairs: Vec<(&str, &str)> = vec![("Hello", "Bonjour"), ("World", "Monde")];

    let en_es_content = common::generate_tmx(&en_es_pairs, "en", "es");
    let en_fr_content = common::generate_tmx(&en_fr_pairs, "en", "fr");

    let tmp_es = temp_file_with_extension("tmx", &en_es_content);
    let tmp_fr = temp_file_with_extension("tmx", &en_fr_content);

    let path_es = tmp_es.path().to_str().unwrap().to_owned();
    let path_fr = tmp_fr.path().to_str().unwrap().to_owned();

    let file_info_es = make_file_info(&path_es);
    let file_info_fr = make_file_info(&path_fr);

    let config = DownloadConfig::default();

    let ingester_es = Ingester::new(pool.clone(), config); // clone: PgPool is Arc-based
    let ingester_fr = Ingester::new(pool.clone(), config); // clone: PgPool is Arc-based

    let handle_es = tokio::spawn(async move {
        ingester_es
            .ingest_from_file(&path_es, &file_info_es, "en", "es")
            .await
            .unwrap();
    });

    let handle_fr = tokio::spawn(async move {
        ingester_fr
            .ingest_from_file(&path_fr, &file_info_fr, "en", "fr")
            .await
            .unwrap();
    });

    tokio::try_join!(handle_es, handle_fr).unwrap();

    let total = common::count_translations(&pool).await;
    assert_eq!(total, 4);

    let lang_count = common::count_languages(&pool).await;
    assert_eq!(lang_count, 3);
}
