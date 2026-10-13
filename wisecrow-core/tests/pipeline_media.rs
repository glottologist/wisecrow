//! Ranked single-word rows still missing a live clip or a recorded picture miss.

use std::path::{Path, PathBuf};

use sqlx::PgPool;
use wisecrow::media::cache::MediaCache;
use wisecrow::media::fingerprint::MediaFingerprint;
use wisecrow::media::{load_media_subject, MediaType};
use wisecrow::pipeline::{next_media_batch, record_picture_miss, ProbeProfile};

const NATIVE: &str = "en";
const FOREIGN: &str = "zzmedia";

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

async fn seed_translation(
    pool: &PgPool,
    from_phrase: &str,
    to_phrase: &str,
    corpus_frequency: Option<i32>,
) -> i32 {
    let (id,): (i32,) = sqlx::query_as(
        "INSERT INTO translations
            (from_language_id, to_language_id, from_phrase, to_phrase, corpus_frequency)
         SELECT fl.id, tl.id, $1, $2, $3
         FROM languages fl, languages tl
         WHERE fl.code = $4 AND tl.code = $5
         ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
         DO UPDATE SET corpus_frequency = EXCLUDED.corpus_frequency
         RETURNING id",
    )
    .bind(from_phrase)
    .bind(to_phrase)
    .bind(corpus_frequency)
    .bind(NATIVE)
    .bind(FOREIGN)
    .fetch_one(pool)
    .await
    .expect("seed translation");
    id
}

fn cache_file(
    cache_dir: &Path,
    translation_id: i32,
    media_type: MediaType,
    fingerprint: &MediaFingerprint,
) -> PathBuf {
    cache_dir.join(media_type.as_str()).join(format!(
        "{translation_id}-{}.{}",
        fingerprint.as_str(),
        media_type.extension()
    ))
}

async fn publish_audio(
    pool: &PgPool,
    cache_dir: &Path,
    translation_id: i32,
    fingerprint: &MediaFingerprint,
) {
    let path = cache_file(cache_dir, translation_id, MediaType::Audio, fingerprint);
    tokio::fs::write(&path, b"audio").await.expect("audio file");
    sqlx::query(
        "INSERT INTO media_cache (translation_id, media_type, file_path, source_fingerprint)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (translation_id, media_type)
         DO UPDATE SET file_path = $3, source_fingerprint = $4",
    )
    .bind(translation_id)
    .bind(MediaType::Audio.as_str())
    .bind(path.to_str().expect("utf8 path"))
    .bind(fingerprint.as_str())
    .execute(pool)
    .await
    .expect("audio row");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn the_batch_is_only_ranked_words_still_missing_media() {
    let pool = test_pool().await;
    sqlx::query(
        "INSERT INTO languages (code, name) VALUES ('en', 'English'), ('zzmedia', 'Pipeline media')
         ON CONFLICT (code) DO NOTHING",
    )
    .execute(&pool)
    .await
    .expect("languages");
    sqlx::query(
        "DELETE FROM translations t
          USING languages fl, languages tl
          WHERE t.from_language_id = fl.id AND t.to_language_id = tl.id
            AND fl.code = 'en' AND tl.code = 'zzmedia'",
    )
    .execute(&pool)
    .await
    .expect("clear pair");

    let ready = seed_translation(&pool, "house", "casa", Some(5)).await;
    let missing = seed_translation(&pool, "dog", "perro", Some(4)).await;
    let linked = seed_translation(&pool, "good day", "linked", Some(3)).await;
    let sentence = seed_translation(&pool, "a long sentence", "gato", None).await;

    let (phrase_id,): (i32,) = sqlx::query_as(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         SELECT id, 'good day', 2, 1 FROM languages WHERE code = 'zzmedia'
         ON CONFLICT (language_id, phrase) DO UPDATE SET sentence_count = EXCLUDED.sentence_count
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .expect("phrase");
    sqlx::query(
        "INSERT INTO phrase_translations (phrase_id, native_language_id, translation, translation_id)
         SELECT $1, id, 'good day', $2 FROM languages WHERE code = 'en'
         ON CONFLICT (phrase_id, native_language_id)
         DO UPDATE SET translation_id = EXCLUDED.translation_id",
    )
    .bind(phrase_id)
    .bind(linked)
    .execute(&pool)
    .await
    .expect("phrase link");

    let dir = tempfile::tempdir().expect("cache dir");
    let cache = MediaCache::with_cache_dir(pool.clone(), dir.path()).expect("cache");
    let subject = load_media_subject(&pool, ready)
        .await
        .expect("subject")
        .expect("ready subject");
    let audio = subject.audio_fingerprint("edge", "test-voice", 1);
    let image = subject
        .image_fingerprint(&["unsplash"], 1)
        .expect("unglossed word has a picture query");
    publish_audio(&pool, dir.path(), ready, &audio).await;
    record_picture_miss(&pool, image.as_str())
        .await
        .expect("picture miss");

    let providers: &[&str] = &["unsplash"];
    let profile = ProbeProfile {
        audio_backend: "edge",
        audio_voice: "test-voice",
        audio_format: 1,
        check_audio: true,
        provider_ids: providers,
        image_format: 1,
        check_image: true,
        image_providers_known: true,
    };
    let batch = next_media_batch(&pool, NATIVE, FOREIGN, &cache, &profile)
        .await
        .expect("batch");
    assert_eq!(batch, vec![missing]);
    assert!(!batch.contains(&ready));
    assert!(!batch.contains(&linked));
    assert!(!batch.contains(&sentence));

    let pictured = load_media_subject(&pool, missing)
        .await
        .expect("missing subject")
        .expect("present");
    let picture = pictured
        .image_fingerprint(providers, 1)
        .expect("foreign word is the query");
    let picture_path = cache_file(dir.path(), missing, MediaType::Image, &picture);
    tokio::fs::write(&picture_path, b"picture")
        .await
        .expect("picture file");
    sqlx::query(
        "INSERT INTO media_cache (translation_id, media_type, file_path, source_fingerprint)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(missing)
    .bind(MediaType::Image.as_str())
    .bind(picture_path.to_str().expect("utf8 path"))
    .bind(picture.as_str())
    .execute(&pool)
    .await
    .expect("picture row");

    let found = wisecrow::media::cached_picture(&cache, missing, &pictured, Some(providers))
        .await
        .expect("cached picture");
    assert!(
        matches!(found, wisecrow::media::CachedPicture::Hit { .. }),
        "a stored picture for an unglossed word is served, not answered as no picture"
    );
}
