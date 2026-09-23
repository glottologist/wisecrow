//! The version-2 grammar endpoints a device syncs against.
//!
//! What matters here is what a unit test cannot see: that a page carries the
//! item a device must hold and a cursor it may safely resume from, that the
//! mastery feed answers for the authenticated learner alone, and that a batch
//! uploaded twice is applied once and acknowledged both times.

#![cfg(feature = "server")]

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sqlx::types::chrono::Utc;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wisecrow::auth::hash_password;
use wisecrow_dto::{
    GrammarAttemptBatchResponseDto, GrammarBankChangePageDto, GrammarMasteryChangePageDto,
    OfflineAttemptStatusDto, MOBILE_PROTOCOL_VERSION_V2,
};
use wisecrow_web::server::auth::issue_session;
use wisecrow_web::server::{build_router, init_pool, pool};

const EMAIL: &str = "grammar-sync@test.local";
const SLUG_PREFIX: &str = "grammar-sync-";
const ANSWER: &str = "estoy";

async fn post(path: &str, request: Value, token: &str) -> Response {
    let request = Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, ["Bearer ", token].concat())
        .body(Body::from(request.to_string()))
        .expect("request");
    build_router().oneshot(request).await.expect("response")
}

async fn response_json<T: DeserializeOwned>(response: Response, label: &str) -> T {
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect(label);
    assert_eq!(
        status,
        StatusCode::OK,
        "{label}: {}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice(&body).unwrap_or_else(|error| {
        panic!("{label}: {error}; body: {}", String::from_utf8_lossy(&body))
    })
}

/// Empties this suite's learner and the Spanish syllabus.
///
/// The whole language goes, as the sibling suite's cleanup does and for the
/// same reason: the feeds are asserted by content, and a point left behind by
/// another suite would appear in them. The runner serialises tests.
async fn cleanup(db: &PgPool) {
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(EMAIL)
        .execute(db)
        .await
        .expect("user cleanup");
    sqlx::query(
        "DELETE FROM grammar_rules gr USING languages l
         WHERE l.id = gr.language_id AND l.code = 'es'",
    )
    .execute(db)
    .await
    .expect("rule cleanup");
    sqlx::query("DELETE FROM quiz_item_changes WHERE language_code = 'es'")
        .execute(db)
        .await
        .expect("feed cleanup");
}

async fn seed_item(db: &PgPool, slug: &str, status: &str) -> (i32, i32) {
    let language_id: i32 = sqlx::query_scalar(
        "INSERT INTO languages (code, name) VALUES ('es', 'Spanish')
         ON CONFLICT (code) DO UPDATE SET name = EXCLUDED.name RETURNING id",
    )
    .fetch_one(db)
    .await
    .expect("language");

    let rule_id: i32 = sqlx::query_scalar(
        "INSERT INTO grammar_rules (language_id, cefr_level_id, slug, title, explanation, source)
         SELECT $1, cl.id, $2, 'Ser vs estar', 'Permanent against temporary.', 'reference'
         FROM cefr_levels cl WHERE cl.code = 'A1'
         RETURNING id",
    )
    .bind(language_id)
    .bind(slug)
    .fetch_one(db)
    .await
    .expect("rule");

    let item_id: i32 = sqlx::query_scalar(
        "INSERT INTO quiz_items (rule_id, kind, prompt, answer, accepted, status, content_sha256)
         VALUES ($1, 'cloze', 'Yo ___ cansado.', $2, '[\"estoy mal\"]'::jsonb, $3, $4)
         RETURNING id",
    )
    .bind(rule_id)
    .bind(ANSWER)
    .bind(status)
    .bind(format!("{:0>64}", format!("{rule_id:x}")))
    .fetch_one(db)
    .await
    .expect("item");

    (rule_id, item_id)
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn version_two_feeds_paginate_and_uploads_apply_once() {
    std::env::set_var(
        "WISECROW__DB_URL",
        std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".into()),
    );
    init_pool().await.expect("init pool");
    let db = pool().expect("pool");
    cleanup(db).await;

    let hash = hash_password("hunter2").expect("hash");
    let user_id: i32 = sqlx::query_scalar(
        "INSERT INTO users (display_name, email, password_hash, is_admin)
         VALUES ('Sync', $1, $2, false) RETURNING id",
    )
    .bind(EMAIL)
    .bind(&hash)
    .fetch_one(db)
    .await
    .expect("user");
    let token = issue_session(db, user_id).await.expect("session");

    let device_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO mobile_devices (user_id, id, display_name) VALUES ($1, $2, 'Test phone')",
    )
    .bind(user_id)
    .bind(device_id)
    .execute(db)
    .await
    .expect("device");

    let (_, active_item) = seed_item(db, &format!("{SLUG_PREFIX}active"), "active").await;
    let (_, candidate_item) = seed_item(db, &format!("{SLUG_PREFIX}candidate"), "candidate").await;

    // The bank feed reports both rows, but hands over only the promoted one:
    // a device must be told to drop an item it should no longer hold.
    let page: GrammarBankChangePageDto = response_json(
        post(
            "/api/mobile/v2/grammar/bank/changes",
            json!({"request": {"protocol_version": MOBILE_PROTOCOL_VERSION_V2, "language": "es",
                   "cursor": 0, "limit": 1}}),
            &token,
        )
        .await,
        "first bank page",
    )
    .await;
    assert_eq!(page.changes.len(), 1, "the limit is honoured");
    assert!(page.has_more, "a second change is waiting");
    let first = &page.changes[0];
    assert_eq!(first.item_id, active_item);
    let item = first.item.as_ref().expect("a promoted item travels");
    assert_eq!(item.answer.as_deref(), Some(ANSWER));
    assert_eq!(item.accepted, vec![String::from("estoy mal")]);
    assert_eq!(item.language, "es");

    let rest: GrammarBankChangePageDto = response_json(
        post(
            "/api/mobile/v2/grammar/bank/changes",
            json!({"request": {"protocol_version": MOBILE_PROTOCOL_VERSION_V2, "language": "es",
                   "cursor": page.next_cursor, "limit": 100}}),
            &token,
        )
        .await,
        "second bank page",
    )
    .await;
    assert_eq!(rest.changes.len(), 1);
    assert_eq!(rest.changes[0].item_id, candidate_item);
    assert!(
        rest.changes[0].item.is_none(),
        "an unpromoted item is reported but never handed over"
    );

    // A batch of one answer, uploaded twice.
    let session_id = Uuid::new_v4();
    let event_id = Uuid::new_v4();
    let batch = json!({"request": {
        "protocol_version": MOBILE_PROTOCOL_VERSION_V2,
        "device_id": device_id,
        "attempts": [{
            "session_id": session_id,
            "event_id": event_id,
            "item_id": active_item,
            "revision": 1,
            "answer": "ESTOY",
            "chose_option": false,
            "hint_shown": false,
            "ordinal": 1,
            "occurred_at": Utc::now(),
            "language": "es",
            "level": "A1"
        }]
    }});

    let accepted: GrammarAttemptBatchResponseDto = response_json(
        post(
            "/api/mobile/v2/grammar/attempts/upload",
            batch.clone(),
            &token,
        )
        .await,
        "first upload",
    )
    .await;
    match &accepted.results[0] {
        OfflineAttemptStatusDto::Accepted(verdict) => {
            assert_eq!(verdict.event_id, event_id);
            assert!(verdict.correct, "the server folds case and accents itself");
        }
        other => panic!("expected an acceptance, got {other:?}"),
    }

    let again: GrammarAttemptBatchResponseDto = response_json(
        post("/api/mobile/v2/grammar/attempts/upload", batch, &token).await,
        "second upload",
    )
    .await;
    match &again.results[0] {
        OfflineAttemptStatusDto::Duplicate(verdict) => {
            assert!(verdict.correct, "the stored verdict is repeated");
        }
        other => panic!("expected a duplicate, got {other:?}"),
    }

    let attempts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM grammar_attempts WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(db)
            .await
            .expect("attempt count");
    assert_eq!(attempts, 1, "an answer uploaded twice is recorded once");

    // The mastery the upload produced reaches the device through its own feed.
    let mastery: GrammarMasteryChangePageDto = response_json(
        post(
            "/api/mobile/v2/grammar/mastery/changes",
            json!({"request": {"protocol_version": MOBILE_PROTOCOL_VERSION_V2, "cursor": 0, "limit": 100}}),
            &token,
        )
        .await,
        "mastery page",
    )
    .await;
    let state = mastery
        .changes
        .iter()
        .find_map(|change| change.mastery.as_ref())
        .expect("the answer moved a point");
    assert_eq!(state.reps, 1);
    assert_eq!(state.attempts, 1);
    assert_eq!(state.accuracy, Some(1.0));
    assert!(mastery.next_cursor > 0);

    // A protocol version this endpoint does not speak is refused outright.
    let refused = post(
        "/api/mobile/v2/grammar/mastery/changes",
        json!({"request": {"protocol_version": 1, "cursor": 0, "limit": 10}}),
        &token,
    )
    .await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);

    cleanup(db).await;
}
