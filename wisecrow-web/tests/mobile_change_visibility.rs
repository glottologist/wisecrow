//! Commit order, for the corpus and card change feeds.
//!
//! `BIGSERIAL` hands out its number when a row is inserted rather than when
//! the transaction commits, so two writers commit out of order as a matter of
//! routine. A reader that serves the higher number first invites the client to
//! advance past the lower one, which it will then never ask for again. Feeds
//! added by migration 031 were born with the guard against this; migration 032
//! gave it to these two, which predate it, and these tests are what hold it.
//!
//! This lives apart from `mobile_corpus_sync.rs` because `init_pool` stores the
//! pool in a `OnceCell` that refuses a second call, and each integration test
//! file is its own binary. Both feeds are exercised from a single test for the
//! same reason its neighbour is one test: the pool is a process-wide global,
//! while `#[tokio::test]` builds a runtime per test and drops it at the end, so
//! a second test inherits pooled connections whose driver tasks have already
//! died with the first runtime and times out waiting for them.
#![cfg(feature = "server")]

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use wisecrow_dto::{
    CardChangeDto, CardChangePageDto, CorpusChangePageDto, MOBILE_PROTOCOL_VERSION,
};
use wisecrow_web::server::{build_router, init_pool, pool};

const EMAIL: &str = "mobile-visibility@test.local";
const PHRASE_PREFIX: &str = "mobile-visibility-";

async fn shared_pool() -> &'static PgPool {
    std::env::set_var(
        "WISECROW__DB_URL",
        std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".into()),
    );
    let _ = init_pool().await;
    pool().expect("pool")
}

async fn post(path: &str, request: Value, bearer: &str) -> Response {
    build_router()
        .oneshot(
            Request::post(path)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, ["Bearer ", bearer].concat())
                .body(Body::from(request.to_string()))
                .expect("request"),
        )
        .await
        .expect("response")
}

async fn page<T: DeserializeOwned>(path: &str, request: Value, token: &str, label: &str) -> T {
    let response = post(path, request, token).await;
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(
        status,
        StatusCode::OK,
        "{label}: {:?}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice(&body).unwrap_or_else(|e| panic!("{label}: {e}"))
}

struct Fixture {
    user_id: i32,
    token: String,
    first: i32,
    second: i32,
}

async fn cleanup(db: &PgPool) {
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(EMAIL)
        .execute(db)
        .await
        .expect("user cleanup");
    sqlx::query("DELETE FROM translations WHERE from_phrase LIKE $1")
        .bind([PHRASE_PREFIX, "%"].concat())
        .execute(db)
        .await
        .expect("translation cleanup");
}

async fn setup(db: &PgPool) -> Fixture {
    cleanup(db).await;
    let en = language(db, "en", "English").await;
    let de = language(db, "de", "German").await;
    let user_id = sqlx::query_scalar(
        "INSERT INTO users (display_name, email, is_admin)
         VALUES ('Visibility', $1, false) RETURNING id",
    )
    .bind(EMAIL)
    .fetch_one(db)
    .await
    .expect("user");
    let token = wisecrow_web::server::auth::issue_session(db, user_id)
        .await
        .expect("session");
    let first = translation(db, en, de, "mobile-visibility-alpha", "eins").await;
    let second = translation(db, en, de, "mobile-visibility-beta", "zwei").await;
    for translation_id in [first, second] {
        sqlx::query("INSERT INTO cards (user_id, translation_id) VALUES ($1, $2)")
            .bind(user_id)
            .bind(translation_id)
            .execute(db)
            .await
            .expect("card");
    }
    Fixture {
        user_id,
        token,
        first,
        second,
    }
}

async fn language(db: &PgPool, code: &str, name: &str) -> i32 {
    sqlx::query_scalar(
        "INSERT INTO languages (code, name) VALUES ($1, $2)
         ON CONFLICT (code) DO UPDATE SET name = EXCLUDED.name
         RETURNING id",
    )
    .bind(code)
    .bind(name)
    .fetch_one(db)
    .await
    .expect("language")
}

async fn translation(db: &PgPool, from: i32, to: i32, from_phrase: &str, to_phrase: &str) -> i32 {
    sqlx::query_scalar(
        "INSERT INTO translations (
             from_language_id, from_phrase, to_language_id, to_phrase, frequency
         ) VALUES ($1, $2, $3, $4, 1)
         RETURNING id",
    )
    .bind(from)
    .bind(from_phrase)
    .bind(to)
    .bind(to_phrase)
    .fetch_one(db)
    .await
    .expect("translation")
}

async fn card_page(fixture: &Fixture, cursor: i64, label: &str) -> CardChangePageDto {
    page(
        "/api/mobile/cards/changes",
        json!({
            "request": {
                "protocol_version": MOBILE_PROTOCOL_VERSION,
                "cursor": cursor,
                "limit": 500
            }
        }),
        &fixture.token,
        label,
    )
    .await
}

async fn corpus_page(fixture: &Fixture, cursor: i64, label: &str) -> CorpusChangePageDto {
    page(
        "/api/mobile/corpus/changes",
        json!({
            "request": {
                "protocol_version": MOBILE_PROTOCOL_VERSION,
                "pair": { "native_lang": "en", "foreign_lang": "de" },
                "cursor": cursor,
                "limit": 500
            }
        }),
        &fixture.token,
        label,
    )
    .await
}

fn card_ids(changes: &[CardChangeDto]) -> Vec<i32> {
    changes
        .iter()
        .map(|change| match change {
            CardChangeDto::Upsert { card, .. } => card.translation_id,
            CardChangeDto::Delete { translation_id, .. } => *translation_id,
        })
        .collect()
}

/// A card change written first and committed last must not be stepped over by
/// a cursor that has already seen the change written second.
async fn assert_the_card_feed_withholds_then_serves(db: &PgPool) {
    let fixture = setup(db).await;
    let cursor = card_page(&fixture, 0, "initial cards").await.next_cursor;

    // Writes first, so it takes the lower sequence number, and commits last.
    let mut late = db.begin().await.expect("late transaction");
    sqlx::query("UPDATE cards SET reps = 11 WHERE user_id = $1 AND translation_id = $2")
        .bind(fixture.user_id)
        .bind(fixture.first)
        .execute(&mut *late)
        .await
        .expect("late update");

    let mut early = db.begin().await.expect("early transaction");
    sqlx::query("UPDATE cards SET reps = 22 WHERE user_id = $1 AND translation_id = $2")
        .bind(fixture.user_id)
        .bind(fixture.second)
        .execute(&mut *early)
        .await
        .expect("early update");
    early.commit().await.expect("early commit");

    let withheld = card_page(&fixture, cursor, "withheld cards").await;
    assert!(
        withheld.changes.is_empty(),
        "a change whose predecessor is still in flight must wait for it, got {:?}",
        card_ids(&withheld.changes)
    );
    assert_eq!(
        withheld.next_cursor, cursor,
        "an empty page must leave the cursor where it was"
    );

    late.commit().await.expect("late commit");

    let served = card_page(&fixture, cursor, "served cards").await;
    let seen = card_ids(&served.changes);
    assert!(
        seen.contains(&fixture.first),
        "the late commit must still arrive, saw {seen:?}"
    );
    assert!(
        seen.contains(&fixture.second),
        "and so must the early one, saw {seen:?}"
    );
    cleanup(db).await;
}

/// The same property for the corpus feed, whose rows are keyed by language
/// pair rather than by learner.
async fn assert_the_corpus_feed_withholds_then_serves(db: &PgPool) {
    let fixture = setup(db).await;
    let cursor = corpus_page(&fixture, 0, "initial corpus").await.next_cursor;

    let mut late = db.begin().await.expect("late transaction");
    sqlx::query("UPDATE translations SET frequency = 11 WHERE id = $1")
        .bind(fixture.first)
        .execute(&mut *late)
        .await
        .expect("late update");

    let mut early = db.begin().await.expect("early transaction");
    sqlx::query("UPDATE translations SET frequency = 22 WHERE id = $1")
        .bind(fixture.second)
        .execute(&mut *early)
        .await
        .expect("early update");
    early.commit().await.expect("early commit");

    let withheld = corpus_page(&fixture, cursor, "withheld corpus").await;
    let withheld_ids: Vec<i32> = withheld
        .changes
        .iter()
        .map(|change| change.translation_id)
        .collect();
    assert!(
        withheld.changes.is_empty(),
        "a change whose predecessor is still in flight must wait for it, got {withheld_ids:?}"
    );
    assert_eq!(
        withheld.next_cursor, cursor,
        "an empty page must leave the cursor where it was"
    );

    late.commit().await.expect("late commit");

    let served = corpus_page(&fixture, cursor, "served corpus").await;
    let seen: Vec<i32> = served
        .changes
        .iter()
        .map(|change| change.translation_id)
        .collect();
    assert!(
        seen.contains(&fixture.first),
        "the late commit must still arrive, saw {seen:?}"
    );
    assert!(
        seen.contains(&fixture.second),
        "and so must the early one, saw {seen:?}"
    );
    cleanup(db).await;
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn changes_committed_late_are_never_skipped_by_the_cursor() {
    let db = shared_pool().await;
    assert_the_card_feed_withholds_then_serves(db).await;
    assert_the_corpus_feed_withholds_then_serves(db).await;
}
