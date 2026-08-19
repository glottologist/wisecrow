#![cfg(feature = "server")]

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sqlx::types::chrono::{DateTime, Utc};
use sqlx::PgPool;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;
use wisecrow_dto::{
    CardChangeDto, CardChangePageDto, CardDto, CardSnapshotDto, CardStatusDto,
    DeviceRegistrationRequestDto, RegisteredDeviceDto, ReviewBatchRequestDto,
    ReviewBatchResponseDto, ReviewEventDto, ReviewEventStatusDto, ReviewRatingDto,
    MOBILE_PROTOCOL_VERSION,
};
use wisecrow_learning::srs::{
    CardState, CardStatus, FsrsScheduler, ReviewEvent, ReviewRating, Scheduler,
};
use wisecrow_web::server::{build_router, init_pool, pool};

const EMAIL: &str = "web-mobile-convergence@test.local";
const PHRASE: &str = "web mobile convergence source";

struct Fixture {
    user_id: i32,
    translation_id: i32,
    card_id: i32,
    session_id: i32,
    device_id: Uuid,
    web_token: String,
    mobile_token: String,
    baseline_at: DateTime<Utc>,
}

async fn post(path: &str, body: Value, token: &str) -> Response {
    let request = Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, ["Bearer ", token].concat())
        .body(Body::from(body.to_string()))
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
    sqlx::query("DELETE FROM translations WHERE from_phrase = $1")
        .bind(PHRASE)
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

async fn seed_translation(db: &PgPool) -> i32 {
    let source = language_id(db, "de", "German").await;
    let target = language_id(db, "en", "English").await;
    sqlx::query_scalar(
        "INSERT INTO translations (
             from_language_id, from_phrase, to_language_id, to_phrase, frequency
         ) VALUES ($1, $2, $3, 'web mobile convergence target', 20)
         RETURNING id",
    )
    .bind(source)
    .bind(PHRASE)
    .bind(target)
    .fetch_one(db)
    .await
    .expect("translation")
}

async fn setup_fixture(db: &PgPool) -> Fixture {
    cleanup(db).await;
    let baseline_at =
        DateTime::from_timestamp(Utc::now().timestamp() - 60, 0).expect("baseline timestamp");
    let user_id: i32 = sqlx::query_scalar(
        "INSERT INTO users (display_name, email, is_admin)
         VALUES ('Convergence', $1, false) RETURNING id",
    )
    .bind(EMAIL)
    .fetch_one(db)
    .await
    .expect("user");
    let translation_id = seed_translation(db).await;
    let card_id = seed_card(db, user_id, translation_id, baseline_at).await;
    let session_id = seed_session(db, user_id, card_id).await;
    let device_id = Uuid::from_u128(9_001);
    let web_token = wisecrow_web::server::auth::issue_session(db, user_id)
        .await
        .expect("web bearer session");
    let mobile_token = wisecrow_web::server::auth::issue_session(db, user_id)
        .await
        .expect("mobile bearer session");
    Fixture {
        user_id,
        translation_id,
        card_id,
        session_id,
        device_id,
        web_token,
        mobile_token,
        baseline_at,
    }
}

async fn seed_card(
    db: &PgPool,
    user_id: i32,
    translation_id: i32,
    baseline_at: DateTime<Utc>,
) -> i32 {
    let card_id = sqlx::query_scalar(
        "INSERT INTO cards (user_id, translation_id, due)
         VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(user_id)
    .bind(translation_id)
    .bind(baseline_at)
    .fetch_one(db)
    .await
    .expect("card");
    sqlx::query(
        "INSERT INTO card_review_baselines (
             user_id, translation_id, stability, difficulty, elapsed_days,
             scheduled_days, reps, lapses, state, last_review, due, captured_at
         ) VALUES ($1, $2, 0, 0, 0, 0, 0, 0, 0, NULL, $3, $3)",
    )
    .bind(user_id)
    .bind(translation_id)
    .bind(baseline_at)
    .execute(db)
    .await
    .expect("baseline");
    card_id
}

async fn seed_session(db: &PgPool, user_id: i32, card_id: i32) -> i32 {
    let session_id = sqlx::query_scalar(
        "INSERT INTO sessions (user_id, native_lang, foreign_lang, deck_size, speed_ms)
         VALUES ($1, 'en', 'de', 1, 3000) RETURNING id",
    )
    .bind(user_id)
    .fetch_one(db)
    .await
    .expect("session");
    sqlx::query("INSERT INTO session_cards (session_id, card_id, position) VALUES ($1, $2, 0)")
        .bind(session_id)
        .bind(card_id)
        .execute(db)
        .await
        .expect("session card");
    session_id
}

async fn register_device(fixture: &Fixture) {
    let request = DeviceRegistrationRequestDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        device_id: fixture.device_id,
        display_name: String::from("Convergence phone"),
    };
    let response = post(
        "/api/mobile/devices/register",
        json!({ "request": request }),
        &fixture.mobile_token,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let _: RegisteredDeviceDto = response_json(response, "registered device").await;
}

async fn apply_web_review(db: &PgPool, fixture: &Fixture) -> DateTime<Utc> {
    let response = post(
        "/api/learn/card/answer",
        json!({
            "session_id": fixture.session_id,
            "card_id": fixture.card_id,
            "rating": ReviewRatingDto::Good,
        }),
        &fixture.web_token,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let card: CardDto = response_json(response, "web review card").await;
    assert_eq!(card.translation_id, fixture.translation_id);
    sqlx::query_scalar(
        "SELECT occurred_at FROM review_events
         WHERE user_id = $1 AND translation_id = $2 AND source = 'web'",
    )
    .bind(fixture.user_id)
    .bind(fixture.translation_id)
    .fetch_one(db)
    .await
    .expect("web review timestamp")
}

fn mobile_events(fixture: &Fixture, web_at: DateTime<Utc>) -> Vec<ReviewEventDto> {
    vec![
        ReviewEventDto {
            event_id: Uuid::from_u128(9_003),
            translation_id: fixture.translation_id,
            rating: ReviewRatingDto::Easy,
            occurred_at: web_at + Duration::from_secs(2),
        },
        ReviewEventDto {
            event_id: Uuid::from_u128(9_002),
            translation_id: fixture.translation_id,
            rating: ReviewRatingDto::Again,
            occurred_at: web_at + Duration::from_secs(1),
        },
    ]
}

async fn apply_mobile_reviews(fixture: &Fixture, events: &[ReviewEventDto]) {
    let request = ReviewBatchRequestDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        device_id: fixture.device_id,
        events: events.to_vec(),
    };
    let response = post(
        "/api/mobile/reviews/upload",
        json!({ "request": request }),
        &fixture.mobile_token,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let applied: ReviewBatchResponseDto = response_json(response, "mobile reviews").await;
    assert!(applied
        .acknowledgements
        .iter()
        .all(|ack| ack.status == ReviewEventStatusDto::Applied));

    let retry = ReviewBatchRequestDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        device_id: fixture.device_id,
        events: events.to_vec(),
    };
    let response = post(
        "/api/mobile/reviews/upload",
        json!({ "request": retry }),
        &fixture.mobile_token,
    )
    .await;
    let retried: ReviewBatchResponseDto = response_json(response, "retried reviews").await;
    assert!(retried
        .acknowledgements
        .iter()
        .all(|ack| ack.status == ReviewEventStatusDto::AlreadyApplied));
}

async fn fetched_card(fixture: &Fixture) -> CardSnapshotDto {
    let response = post(
        "/api/mobile/cards/changes",
        json!({
            "request": { "protocol_version": MOBILE_PROTOCOL_VERSION, "cursor": 0, "limit": 100 }
        }),
        &fixture.mobile_token,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let page: CardChangePageDto = response_json(response, "card changes").await;
    page.changes
        .into_iter()
        .find_map(|change| match change {
            CardChangeDto::Upsert { card, .. } if card.translation_id == fixture.translation_id => {
                Some(card)
            }
            _ => None,
        })
        .expect("converged card change")
}

async fn expected_card(db: &PgPool, fixture: &Fixture) -> CardSnapshotDto {
    let rows: Vec<(Uuid, DateTime<Utc>, i16)> = sqlx::query_as(
        "SELECT event_id, occurred_at, rating FROM review_events
         WHERE user_id = $1 AND translation_id = $2 ORDER BY occurred_at, event_id",
    )
    .bind(fixture.user_id)
    .bind(fixture.translation_id)
    .fetch_all(db)
    .await
    .expect("review events");
    let events = rows
        .into_iter()
        .map(|(event_id, occurred_at, rating)| ReviewEvent {
            event_id,
            occurred_at,
            rating: learning_rating(rating),
        })
        .collect::<Vec<_>>();
    let baseline = CardState::new(fixture.translation_id, fixture.baseline_at, CardStatus::New);
    let replayed = FsrsScheduler
        .replay(&baseline, &events)
        .expect("independent FSRS replay");
    snapshot_from_replay(db, fixture.user_id, replayed).await
}

async fn snapshot_from_replay(db: &PgPool, user_id: i32, replayed: CardState) -> CardSnapshotDto {
    let stability = stored_float(db, replayed.stability).await;
    let difficulty = stored_float(db, replayed.difficulty).await;
    let server_cursor: i64 = sqlx::query_scalar(
        "SELECT MAX(sequence) FROM card_changes WHERE user_id = $1 AND translation_id = $2",
    )
    .bind(user_id)
    .bind(replayed.translation_id)
    .fetch_one(db)
    .await
    .expect("card cursor");
    CardSnapshotDto {
        translation_id: replayed.translation_id,
        stability,
        difficulty,
        elapsed_days: replayed.elapsed_days,
        scheduled_days: replayed.scheduled_days,
        reps: replayed.reps,
        lapses: replayed.lapses,
        state: card_status(replayed.status),
        last_review: replayed.last_review,
        due: replayed.due,
        server_cursor,
    }
}

async fn stored_float(db: &PgPool, value: f64) -> f64 {
    let stored: f32 = sqlx::query_scalar("SELECT $1::double precision::real")
        .bind(value)
        .fetch_one(db)
        .await
        .expect("stored float precision");
    f64::from(stored)
}

fn learning_rating(rating: i16) -> ReviewRating {
    match rating {
        1 => ReviewRating::Again,
        2 => ReviewRating::Hard,
        3 => ReviewRating::Good,
        4 => ReviewRating::Easy,
        _ => panic!("unsupported stored review rating"),
    }
}

const fn card_status(status: CardStatus) -> CardStatusDto {
    match status {
        CardStatus::New => CardStatusDto::New,
        CardStatus::Learning => CardStatusDto::Learning,
        CardStatus::Review => CardStatusDto::Review,
        CardStatus::Relearning => CardStatusDto::Relearning,
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn browser_and_mobile_reviews_converge_across_bearer_sessions() {
    std::env::set_var(
        "WISECROW__DB_URL",
        std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".into()),
    );
    init_pool().await.expect("pool");
    let db = pool().expect("pool");
    let fixture = setup_fixture(db).await;
    assert_ne!(fixture.web_token, fixture.mobile_token);
    register_device(&fixture).await;
    let web_at = apply_web_review(db, &fixture).await;
    let events = mobile_events(&fixture, web_at);
    apply_mobile_reviews(&fixture, &events).await;
    let actual = fetched_card(&fixture).await;
    let expected = expected_card(db, &fixture).await;
    assert_eq!(actual, expected);
    cleanup(db).await;
}
