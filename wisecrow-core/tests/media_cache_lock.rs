use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use wisecrow::media::cache::MediaCache;
use wisecrow::media::fingerprint::MediaFingerprint;
use wisecrow::media::MediaType;

struct Fixture {
    pool: PgPool,
    dir: tempfile::TempDir,
    cache: MediaCache,
    translation_id: i32,
}

impl Fixture {
    async fn new(from_phrase: &str, to_phrase: &str) -> Self {
        let pool = test_pool().await;
        let translation_id = seed_named_translation(&pool, from_phrase, to_phrase).await;
        sqlx::query("DELETE FROM media_cache WHERE translation_id = $1")
            .bind(translation_id)
            .execute(&pool)
            .await
            .expect("cache cleanup");
        let dir = tempfile::tempdir().expect("cache dir");
        let cache = MediaCache::with_cache_dir(pool.clone(), dir.path()).expect("cache init");
        Self {
            pool,
            dir,
            cache,
            translation_id,
        }
    }
}

async fn test_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5432/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url)
        .await
        .expect("Failed to connect to test database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
}

async fn seed_named_translation(pool: &PgPool, from_phrase: &str, to_phrase: &str) -> i32 {
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('gd', 'Gaelic')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("languages seed");
    let (id,): (i32,) = sqlx::query_as(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase)
         SELECT fl.id, tl.id, $1, $2
         FROM languages fl, languages tl WHERE fl.code = 'en' AND tl.code = 'gd'
         ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
         DO UPDATE SET from_phrase = EXCLUDED.from_phrase
         RETURNING id",
    )
    .bind(from_phrase)
    .bind(to_phrase)
    .fetch_one(pool)
    .await
    .expect("translation seed");
    sqlx::query("DELETE FROM media_cache WHERE translation_id = $1")
        .bind(id)
        .execute(pool)
        .await
        .expect("cache cleanup");
    id
}

fn fingerprint_for(media_type: MediaType, tag: &str) -> MediaFingerprint {
    match media_type {
        MediaType::Audio => MediaFingerprint::for_audio(tag, "gd", "edge", "voice-a", 1),
        MediaType::Image => MediaFingerprint::for_image(tag, "gd", &["unsplash"], 1),
    }
}

fn seeded_path(
    cache_dir: &Path,
    translation_id: i32,
    media_type: MediaType,
    fingerprint: Option<&MediaFingerprint>,
) -> PathBuf {
    let name = match fingerprint {
        Some(fingerprint) => format!(
            "{translation_id}-{}.{}",
            fingerprint.as_str(),
            media_type.extension()
        ),
        None => format!("{translation_id}.{}", media_type.extension()),
    };
    cache_dir.join(media_type.as_str()).join(name)
}

async fn seed_file(
    fixture: &Fixture,
    media_type: MediaType,
    fingerprint: Option<&MediaFingerprint>,
    bytes: &[u8],
) -> PathBuf {
    let file_path = seeded_path(
        fixture.dir.path(),
        fixture.translation_id,
        media_type,
        fingerprint,
    );
    if let Some(parent) = file_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .expect("cache type dir");
    }
    tokio::fs::write(&file_path, bytes)
        .await
        .expect("seed cache file");
    let path_str = file_path.to_str().expect("utf8 cache path");
    sqlx::query(
        "INSERT INTO media_cache (translation_id, media_type, file_path, source_fingerprint)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (translation_id, media_type)
         DO UPDATE SET file_path = $3, source_fingerprint = $4, attribution = NULL",
    )
    .bind(fixture.translation_id)
    .bind(media_type.as_str())
    .bind(path_str)
    .bind(fingerprint.map(MediaFingerprint::as_str))
    .execute(&fixture.pool)
    .await
    .expect("seed cache row");
    file_path
}

async fn stored_fingerprint(
    pool: &PgPool,
    translation_id: i32,
    media_type: MediaType,
) -> Option<String> {
    sqlx::query_scalar(
        "SELECT source_fingerprint FROM media_cache
         WHERE translation_id = $1 AND media_type = $2",
    )
    .bind(translation_id)
    .bind(media_type.as_str())
    .fetch_optional(pool)
    .await
    .expect("fingerprint query")
    .flatten()
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn legacy_or_changed_fingerprint_misses_and_fetches() {
    let fixture = Fixture::new("fingerprint miss", "dearbhadh meath").await;
    for media_type in [MediaType::Audio, MediaType::Image] {
        let requested = fingerprint_for(media_type, "current");
        let stale = fingerprint_for(media_type, "stale");
        for seed in [None, Some(stale)] {
            let old_bytes = b"stale-bytes";
            let old_path = seed_file(&fixture, media_type, seed.as_ref(), old_bytes).await;
            let calls = AtomicUsize::new(0);
            let (path, attribution) = fixture
                .cache
                .get_or_fetch_attributed(fixture.translation_id, media_type, &requested, || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok((b"fresh-bytes".to_vec(), Some("credit".to_owned())))
                })
                .await
                .expect("fingerprint miss fetches");

            assert_eq!(calls.load(Ordering::SeqCst), 1, "{media_type:?} fetcher");
            assert_eq!(attribution.as_deref(), Some("credit"));
            assert_ne!(path, old_path);
            assert!(
                path.to_str().expect("utf8").contains(requested.as_str()),
                "published path should include the fingerprint: {}",
                path.display()
            );
            assert_eq!(
                tokio::fs::read(&path).await.expect("published bytes"),
                b"fresh-bytes"
            );
            assert!(
                !old_path.exists(),
                "stale file should be removed after publish: {}",
                old_path.display()
            );
            assert_eq!(
                stored_fingerprint(&fixture.pool, fixture.translation_id, media_type)
                    .await
                    .as_deref(),
                Some(requested.as_str())
            );
        }
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn matching_fingerprint_hits_without_fetch() {
    let fixture = Fixture::new("fingerprint hit", "dearbhadh buail").await;
    for media_type in [MediaType::Audio, MediaType::Image] {
        let fingerprint = fingerprint_for(media_type, "stable");
        let old_path = seed_file(&fixture, media_type, Some(&fingerprint), b"cached").await;
        let (path, _) = fixture
            .cache
            .get_or_fetch_attributed(fixture.translation_id, media_type, &fingerprint, || async {
                panic!("{media_type:?} fetcher must not run on a fingerprint hit")
            })
            .await
            .expect("fingerprint hit");
        assert_eq!(path, old_path);
        assert_eq!(tokio::fs::read(&path).await.expect("hit bytes"), b"cached");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn concurrent_fingerprinted_misses_fetch_once() {
    let fixture = Fixture::new("fingerprint lock", "dearbhadh glas").await;
    let fingerprint = fingerprint_for(MediaType::Audio, "lock");
    let cache_b =
        MediaCache::with_cache_dir(fixture.pool.clone(), fixture.dir.path()).expect("cache b");
    let calls = Arc::new(AtomicUsize::new(0));
    let fetcher = |calls: Arc<AtomicUsize>| {
        move || async move {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(300)).await;
            Ok((vec![0x11u8, 0x22, 0x33], None))
        }
    };

    let (a, b) = tokio::join!(
        fixture.cache.get_or_fetch_attributed(
            fixture.translation_id,
            MediaType::Audio,
            &fingerprint,
            fetcher(Arc::clone(&calls)),
        ),
        cache_b.get_or_fetch_attributed(
            fixture.translation_id,
            MediaType::Audio,
            &fingerprint,
            fetcher(Arc::clone(&calls)),
        ),
    );

    let (path_a, _) = a.expect("first fingerprinted fetch");
    let (path_b, _) = b.expect("second fingerprinted fetch");
    assert_eq!(path_a, path_b);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "provider called more than once"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn new_fingerprint_does_not_expose_partial_old_file() {
    let fixture = Fixture::new("fingerprint atomic", "dearbhadh atomach").await;
    let old_fp = fingerprint_for(MediaType::Audio, "old");
    let new_fp = fingerprint_for(MediaType::Audio, "new");
    let old_path = seed_file(&fixture, MediaType::Audio, Some(&old_fp), &[0xAA; 8192]).await;

    let publish = fixture.cache.get_or_fetch_attributed(
        fixture.translation_id,
        MediaType::Audio,
        &new_fp,
        || async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            Ok((vec![0xBB; 8192], None))
        },
    );
    let probe = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let bytes = tokio::fs::read(&old_path)
            .await
            .expect("old file readable during publish");
        assert_eq!(
            bytes,
            vec![0xAA; 8192],
            "old cache file changed before the new row committed"
        );
    };
    let (published, ()) = tokio::join!(publish, probe);
    let (new_path, _) = published.expect("publish");
    assert_ne!(new_path, old_path);
    assert_eq!(
        tokio::fs::read(&new_path).await.expect("new bytes"),
        vec![0xBB; 8192]
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn database_failure_does_not_point_at_incomplete_file() {
    let fixture = Fixture::new("fingerprint fk", "dearbhadh briste").await;
    let fingerprint = fingerprint_for(MediaType::Audio, "fk");
    sqlx::query("DELETE FROM translations WHERE id = $1")
        .bind(fixture.translation_id)
        .execute(&fixture.pool)
        .await
        .expect("drop translation");

    let result = fixture
        .cache
        .get_or_fetch_attributed(
            fixture.translation_id,
            MediaType::Audio,
            &fingerprint,
            || async { Ok((b"orphan-bytes".to_vec(), None)) },
        )
        .await;
    assert!(result.is_err(), "expected database failure, got {result:?}");

    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT file_path, source_fingerprint FROM media_cache WHERE translation_id = $1",
    )
    .bind(fixture.translation_id)
    .fetch_all(&fixture.pool)
    .await
    .expect("row probe");
    assert!(
        rows.is_empty(),
        "failed publish left a cache row pointing at a file: {rows:?}"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn stale_file_cleanup_errors_are_reported() {
    let fixture = Fixture::new("fingerprint cleanup", "dearbhadh glanadh").await;
    let requested = fingerprint_for(MediaType::Audio, "fresh");
    let stale_dir = fixture
        .dir
        .path()
        .join("audio")
        .join(format!("{}-stale-dir", fixture.translation_id));
    tokio::fs::create_dir_all(&stale_dir)
        .await
        .expect("stale directory");
    tokio::fs::write(stale_dir.join("nested"), b"x")
        .await
        .expect("nested file so remove_file cannot succeed");
    sqlx::query(
        "INSERT INTO media_cache (translation_id, media_type, file_path, source_fingerprint)
         VALUES ($1, 'audio', $2, NULL)",
    )
    .bind(fixture.translation_id)
    .bind(stale_dir.to_str().expect("utf8 stale path"))
    .execute(&fixture.pool)
    .await
    .expect("seed directory as previous path");

    let error = fixture
        .cache
        .get_or_fetch_attributed(
            fixture.translation_id,
            MediaType::Audio,
            &requested,
            || async { Ok((b"fresh-bytes".to_vec(), None)) },
        )
        .await
        .expect_err("cleanup failure must be reported");
    let message = error.to_string();
    assert!(
        message.contains("failed to remove stale cache file"),
        "cleanup error was not reported: {message}"
    );
    assert!(stale_dir.exists(), "undeletable stale path should remain");
    assert_eq!(
        stored_fingerprint(&fixture.pool, fixture.translation_id, MediaType::Audio)
            .await
            .as_deref(),
        Some(requested.as_str()),
        "publication must commit before cleanup"
    );
}
