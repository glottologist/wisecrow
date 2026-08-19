#![cfg(feature = "server")]

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sqlx::types::chrono::Utc;
use sqlx::PgPool;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;
use wisecrow_dto::{
    NbackBatchRequestDto, NbackBatchResponseDto, NbackModeDto, NbackSessionUploadDto,
    NbackTrialResponseDto, NbackUploadStatusDto, ReviewBatchRequestDto, ReviewBatchResponseDto,
    ReviewEventDto, ReviewEventStatusDto, ReviewRatingDto, MOBILE_PROTOCOL_VERSION,
};
use wisecrow_web::server::{build_router, init_pool, pool};

const EMAIL: &str = "mobile-event-sync@test.local";
const PHRASE_PREFIX: &str = "mobile-event-sync-";

struct Fixture {
    user_id: i32,
    device_id: Uuid,
    token: String,
    translation_ids: Vec<i32>,
}

async fn post(path: &str, request: Value, token: &str) -> Response {
    let request = Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, ["Bearer ", token].concat())
        .body(Body::from(request.to_string()))
        .expect("request");
    build_router().oneshot(request).await.expect("response")
}

async fn response_json<T: DeserializeOwned>(response: Response, label: &str) -> T {
    let body = to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect(label);
    serde_json::from_slice(&body).unwrap_or_else(|error| {
        panic!("{label}: {error}; body: {}", String::from_utf8_lossy(&body))
    })
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

async fn language_id(db: &PgPool, code: &str, name: &str) -> i32 {
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

async fn translations(db: &PgPool, de: i32, en: i32) -> Vec<i32> {
    let mut ids = Vec::new();
    for index in 0..8 {
        let id = sqlx::query_scalar(
            "INSERT INTO translations (
                 from_language_id, from_phrase, to_language_id, to_phrase, frequency
             ) VALUES ($1, $2, $3, $4, 10) RETURNING id",
        )
        .bind(de)
        .bind(format!("{PHRASE_PREFIX}de-{index}"))
        .bind(en)
        .bind(format!("event sync en {index}"))
        .fetch_one(db)
        .await
        .expect("translation");
        ids.push(id);
    }
    ids
}

async fn setup_fixture(db: &PgPool) -> Fixture {
    cleanup(db).await;
    let en = language_id(db, "en", "English").await;
    let de = language_id(db, "de", "German").await;
    let user_id: i32 = sqlx::query_scalar(
        "INSERT INTO users (display_name, email, is_admin)
         VALUES ('Mobile Event Sync', $1, false) RETURNING id",
    )
    .bind(EMAIL)
    .fetch_one(db)
    .await
    .expect("user");
    let device_id = Uuid::new_v4();
    sqlx::query("INSERT INTO mobile_devices (user_id, id, display_name) VALUES ($1, $2, 'Phone')")
        .bind(user_id)
        .bind(device_id)
        .execute(db)
        .await
        .expect("device");
    let token = wisecrow_web::server::auth::issue_session(db, user_id)
        .await
        .expect("session");
    Fixture {
        user_id,
        device_id,
        token,
        translation_ids: translations(db, de, en).await,
    }
}

fn review_event(event_id: u128, translation_id: i32, rating: ReviewRatingDto) -> ReviewEventDto {
    ReviewEventDto {
        event_id: Uuid::from_u128(event_id),
        translation_id,
        rating,
        occurred_at: Utc::now() - Duration::from_secs(60),
    }
}

async fn upload_reviews(fixture: &Fixture, events: Vec<ReviewEventDto>) -> Response {
    let request = ReviewBatchRequestDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        device_id: fixture.device_id,
        events,
    };
    post(
        "/api/mobile/reviews/upload",
        json!({ "request": request }),
        &fixture.token,
    )
    .await
}

async fn assert_review_retry_and_partial_rejection(db: &PgPool, fixture: &Fixture) {
    let first = review_event(1, fixture.translation_ids[0], ReviewRatingDto::Good);
    let response = upload_reviews(fixture, vec![first.clone()]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let applied: ReviewBatchResponseDto = response_json(response, "applied review").await;
    assert_eq!(
        applied.acknowledgements[0].status,
        ReviewEventStatusDto::Applied
    );

    let response = upload_reviews(fixture, vec![first.clone()]).await;
    let retried: ReviewBatchResponseDto = response_json(response, "retried review").await;
    assert_eq!(
        retried.acknowledgements[0].status,
        ReviewEventStatusDto::AlreadyApplied
    );
    assert_eq!(applied.cards, retried.cards);

    let accepted = review_event(2, fixture.translation_ids[1], ReviewRatingDto::Hard);
    let rejected = review_event(3, i32::MAX, ReviewRatingDto::Easy);
    let response = upload_reviews(fixture, vec![accepted, rejected]).await;
    let mixed: ReviewBatchResponseDto = response_json(response, "mixed reviews").await;
    assert_eq!(
        mixed.acknowledgements[0].status,
        ReviewEventStatusDto::Applied
    );
    assert!(matches!(
        mixed.acknowledgements[1].status,
        ReviewEventStatusDto::Rejected { .. }
    ));
    assert_eq!(review_count(db, fixture.user_id, "mobile").await, 2);
}

async fn assert_review_collision(fixture: &Fixture) {
    let collision = review_event(1, fixture.translation_ids[0], ReviewRatingDto::Easy);
    let response = upload_reviews(fixture, vec![collision]).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

fn nback_session(fixture: &Fixture, session_id: Uuid) -> NbackSessionUploadDto {
    let completed_at = Utc::now() - Duration::from_secs(5);
    NbackSessionUploadDto {
        client_session_id: session_id,
        pair: wisecrow_dto::LanguagePairDto {
            native_lang: String::from("en"),
            foreign_lang: String::from("de"),
        },
        mode: NbackModeDto::AudioWritten,
        n_level: 2,
        interval_ms: 4_000,
        seed: 42,
        vocabulary_translation_ids: fixture.translation_ids.clone(),
        responses: (1..=5)
            .map(|trial_number| NbackTrialResponseDto {
                trial_number,
                audio_response: Some(false),
                visual_response: Some(false),
                response_time_ms: 100,
            })
            .collect(),
        started_at: completed_at - Duration::from_secs(25),
        completed_at,
    }
}

async fn upload_nback(fixture: &Fixture, sessions: Vec<NbackSessionUploadDto>) -> Response {
    let request = NbackBatchRequestDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        device_id: fixture.device_id,
        sessions,
    };
    post(
        "/api/mobile/nback/upload",
        json!({ "request": request }),
        &fixture.token,
    )
    .await
}

async fn review_count(db: &PgPool, user_id: i32, source: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM review_events WHERE user_id = $1 AND source = $2")
        .bind(user_id)
        .bind(source)
        .fetch_one(db)
        .await
        .expect("review count")
}

async fn nback_counts(db: &PgPool, user_id: i32) -> (i64, i64) {
    let sessions = sqlx::query_scalar("SELECT COUNT(*) FROM dnb_sessions WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(db)
        .await
        .expect("session count");
    let uploads =
        sqlx::query_scalar("SELECT COUNT(*) FROM mobile_nback_uploads WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(db)
            .await
            .expect("upload count");
    (sessions, uploads)
}

async fn card_repetitions(db: &PgPool, user_id: i32) -> i64 {
    sqlx::query_scalar("SELECT COALESCE(SUM(reps), 0) FROM cards WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(db)
        .await
        .expect("card repetitions")
}

async fn assert_nback_retry_and_rejection(db: &PgPool, fixture: &Fixture) {
    let session_id = Uuid::from_u128(100);
    let valid = nback_session(fixture, session_id);
    let mut invalid = nback_session(fixture, Uuid::from_u128(101));
    invalid.vocabulary_translation_ids.truncate(7);
    let response = upload_nback(fixture, vec![valid.clone(), invalid]).await;
    assert_eq!(response.status(), StatusCode::OK);
    let applied: NbackBatchResponseDto = response_json(response, "applied n-back").await;
    assert_eq!(
        applied.acknowledgements[0].status,
        NbackUploadStatusDto::Applied
    );
    assert!(matches!(
        applied.acknowledgements[1].status,
        NbackUploadStatusDto::Rejected { .. }
    ));
    assert_eq!(nback_counts(db, fixture.user_id).await, (1, 1));
    let feedback_count = review_count(db, fixture.user_id, "nback").await;
    let repetitions = card_repetitions(db, fixture.user_id).await;
    assert!(feedback_count > 0);

    let response = upload_nback(fixture, vec![valid.clone()]).await;
    let retried: NbackBatchResponseDto = response_json(response, "retried n-back").await;
    assert_eq!(
        retried.acknowledgements[0].status,
        NbackUploadStatusDto::AlreadyApplied
    );
    assert_eq!(
        applied.acknowledgements[0].result,
        retried.acknowledgements[0].result
    );
    assert_eq!(nback_counts(db, fixture.user_id).await, (1, 1));
    assert_eq!(
        review_count(db, fixture.user_id, "nback").await,
        feedback_count
    );
    assert_eq!(card_repetitions(db, fixture.user_id).await, repetitions);

    let mut collision = valid;
    collision.responses[0].response_time_ms = 101;
    let response = upload_nback(fixture, vec![collision]).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn mobile_event_uploads_are_idempotent_partial_and_server_scored() {
    std::env::set_var(
        "WISECROW__DB_URL",
        std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".into()),
    );
    init_pool().await.expect("pool");
    let db = pool().expect("pool");
    let fixture = setup_fixture(db).await;
    assert_review_retry_and_partial_rejection(db, &fixture).await;
    assert_review_collision(&fixture).await;
    assert_nback_retry_and_rejection(db, &fixture).await;
    cleanup(db).await;
}
