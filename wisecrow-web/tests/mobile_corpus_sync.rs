#![cfg(feature = "server")]

use std::collections::BTreeMap;

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wisecrow_dto::{
    CardChangeDto, CardChangePageDto, CorpusChangeKindDto, CorpusChangePageDto, CorpusPageDto,
    CorpusTranslationDto, MobileCapabilitiesDto, RegisteredDeviceDto, MOBILE_PROTOCOL_VERSION,
};
use wisecrow_web::server::{build_router, init_pool, pool};

const PRIMARY_EMAIL: &str = "mobile-corpus-primary@test.local";
const OTHER_EMAIL: &str = "mobile-corpus-other@test.local";
const PHRASE_PREFIX: &str = "mobile-sync-";

async fn post(path: &str, request: Value, bearer: Option<&str>) -> Response {
    let mut builder = Request::post(path).header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = bearer {
        builder = builder.header(header::AUTHORIZATION, ["Bearer ", token].concat());
    }
    build_router()
        .oneshot(
            builder
                .body(Body::from(request.to_string()))
                .expect("request"),
        )
        .await
        .expect("response")
}

async fn response_json<T: DeserializeOwned>(response: Response, label: &str) -> T {
    let body = to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect(label);
    serde_json::from_slice(&body).expect(label)
}

async fn assert_status(response: Response, expected: StatusCode, label: &str) -> Response {
    let actual = response.status();
    if actual != expected {
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("error response");
        panic!(
            "{label}: expected {expected}, got {actual}: {}",
            String::from_utf8_lossy(&body)
        );
    }
    response
}

async fn cleanup(db: &PgPool) {
    sqlx::query("DELETE FROM users WHERE email IN ($1, $2)")
        .bind(PRIMARY_EMAIL)
        .bind(OTHER_EMAIL)
        .execute(db)
        .await
        .expect("user cleanup");
    sqlx::query("DELETE FROM phrases WHERE phrase LIKE $1")
        .bind([PHRASE_PREFIX, "%"].concat())
        .execute(db)
        .await
        .expect("phrase cleanup");
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

async fn create_user(db: &PgPool, email: &str, display_name: &str) -> (i32, String) {
    let user_id = sqlx::query_scalar(
        "INSERT INTO users (display_name, email, is_admin)
         VALUES ($1, $2, false) RETURNING id",
    )
    .bind(display_name)
    .bind(email)
    .fetch_one(db)
    .await
    .expect("user");
    let token = wisecrow_web::server::auth::issue_session(db, user_id)
        .await
        .expect("session");
    (user_id, token)
}

async fn create_translation(
    db: &PgPool,
    from_language_id: i32,
    to_language_id: i32,
    from_phrase: &str,
    to_phrase: &str,
) -> i32 {
    sqlx::query_scalar(
        "INSERT INTO translations (
             from_language_id, from_phrase, to_language_id, to_phrase, frequency
         ) VALUES ($1, $2, $3, $4, 1)
         RETURNING id",
    )
    .bind(from_language_id)
    .bind(from_phrase)
    .bind(to_language_id)
    .bind(to_phrase)
    .fetch_one(db)
    .await
    .expect("translation")
}

fn snapshot_request(after: i32, watermark: Option<i64>, limit: u16) -> Value {
    json!({
        "request": {
            "protocol_version": MOBILE_PROTOCOL_VERSION,
            "pair": { "native_lang": "en", "foreign_lang": "de" },
            "after_translation_id": after,
            "snapshot_watermark": watermark,
            "limit": limit
        }
    })
}

fn corpus_change_request(native: &str, foreign: &str, cursor: i64, limit: u16) -> Value {
    json!({
        "request": {
            "protocol_version": MOBILE_PROTOCOL_VERSION,
            "pair": { "native_lang": native, "foreign_lang": foreign },
            "cursor": cursor,
            "limit": limit
        }
    })
}

fn card_change_request(cursor: i64) -> Value {
    json!({
        "request": {
            "protocol_version": MOBILE_PROTOCOL_VERSION,
            "cursor": cursor,
            "limit": 500
        }
    })
}

async fn promote_phrase(db: &PgPool, de: i32, en: i32, translation_id: i32) {
    let phrase_id: i32 = sqlx::query_scalar(
        "INSERT INTO phrases (language_id, phrase, token_count, sentence_count)
         VALUES ($1, 'mobile-sync-zwei worte', 2, 3) RETURNING id",
    )
    .bind(de)
    .fetch_one(db)
    .await
    .expect("phrase");
    sqlx::query(
        "INSERT INTO phrase_translations (
             phrase_id, native_language_id, translation, translation_id
         ) VALUES ($1, $2, 'two words', $3)",
    )
    .bind(phrase_id)
    .bind(en)
    .bind(translation_id)
    .execute(db)
    .await
    .expect("phrase translation");
}

fn apply_corpus_change(
    state: &mut BTreeMap<i32, CorpusTranslationDto>,
    change: &wisecrow_dto::CorpusChangeDto,
) {
    match change.kind {
        CorpusChangeKindDto::Upsert => {
            let translation = change.translation.clone().expect("upsert payload");
            state.insert(change.translation_id, translation);
        }
        CorpusChangeKindDto::Delete => {
            state.remove(&change.translation_id);
        }
    }
}

async fn current_corpus(db: &PgPool) -> BTreeMap<i32, CorpusTranslationDto> {
    let rows = sqlx::query_as::<_, (i32, String, String, i32, bool)>(
        "SELECT translation.id, translation.from_phrase, translation.to_phrase,
                translation.frequency,
                EXISTS (
                    SELECT 1 FROM phrase_translations
                    WHERE translation_id = translation.id
                )
         FROM translations AS translation
         JOIN languages AS native ON native.id = translation.from_language_id
         JOIN languages AS target_language ON target_language.id = translation.to_language_id
         WHERE native.code = 'en' AND target_language.code = 'de'
           AND translation.from_phrase LIKE 'mobile-sync-%'
         ORDER BY translation.id",
    )
    .fetch_all(db)
    .await
    .expect("current corpus");
    rows.into_iter()
        .map(
            |(translation_id, from_phrase, to_phrase, frequency, is_phrase)| {
                (
                    translation_id,
                    CorpusTranslationDto {
                        translation_id,
                        from_phrase,
                        to_phrase,
                        frequency,
                        is_phrase,
                    },
                )
            },
        )
        .collect()
}

async fn seed_card(db: &PgPool, user_id: i32, translation_id: i32) {
    sqlx::query("INSERT INTO cards (user_id, translation_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(translation_id)
        .execute(db)
        .await
        .expect("card");
}

struct Fixture {
    en: i32,
    de: i32,
    user_id: i32,
    other_user_id: i32,
    token: String,
    other_token: String,
    first: i32,
    second: i32,
    third: i32,
    wrong_pair: i32,
}

async fn setup_fixture(db: &PgPool) -> Fixture {
    cleanup(db).await;
    let en = language_id(db, "en", "English").await;
    let de = language_id(db, "de", "German").await;
    let fr = language_id(db, "fr", "French").await;
    let (user_id, token) = create_user(db, PRIMARY_EMAIL, "Primary").await;
    let (other_user_id, other_token) = create_user(db, OTHER_EMAIL, "Other").await;
    let first = create_translation(db, en, de, "mobile-sync-alpha", "eins").await;
    let second = create_translation(db, en, de, "mobile-sync-beta", "zwei").await;
    let third = create_translation(db, en, de, "mobile-sync-gamma", "zwei worte").await;
    let wrong_pair = create_translation(db, en, fr, "mobile-sync-wrong-pair", "trois").await;
    Fixture {
        en,
        de,
        user_id,
        other_user_id,
        token,
        other_token,
        first,
        second,
        third,
        wrong_pair,
    }
}

async fn assert_capabilities() {
    let response = post("/api/mobile/capabilities", json!({}), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let capabilities: MobileCapabilitiesDto = response_json(response, "capabilities").await;
    assert_eq!(capabilities.protocol_version, MOBILE_PROTOCOL_VERSION);
    assert_eq!(capabilities.max_snapshot_page, 500);
}

async fn register_device(token: &str) -> Uuid {
    let device_id = Uuid::new_v4();
    let response = post(
        "/api/mobile/devices/register",
        json!({
            "request": {
                "protocol_version": MOBILE_PROTOCOL_VERSION,
                "device_id": device_id,
                "display_name": "  Test phone  "
            }
        }),
        Some(token),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let registered: RegisteredDeviceDto = response_json(response, "registered device").await;
    assert_eq!(registered.display_name, "Test phone");
    assert_device_refresh(token, device_id, &registered).await;
    device_id
}

async fn assert_device_refresh(token: &str, device_id: Uuid, first: &RegisteredDeviceDto) {
    let response = post(
        "/api/mobile/devices/register",
        json!({
            "request": {
                "protocol_version": MOBILE_PROTOCOL_VERSION,
                "device_id": device_id,
                "display_name": "Renamed phone"
            }
        }),
        Some(token),
    )
    .await;
    let refreshed: RegisteredDeviceDto = response_json(response, "refreshed device").await;
    assert_eq!(refreshed.created_at, first.created_at);
    assert!(refreshed.last_seen_at >= first.last_seen_at);
    assert_eq!(refreshed.display_name, "Renamed phone");
}

async fn assert_protocol_and_cursor_validation(token: &str) {
    let unsupported = json!({
        "request": { "protocol_version": 2, "cursor": 0, "limit": 1 }
    });
    let response = post("/api/mobile/cards/changes", unsupported, Some(token)).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let negative = json!({
        "request": {
            "protocol_version": MOBILE_PROTOCOL_VERSION,
            "pair": { "native_lang": "en", "foreign_lang": "de" },
            "cursor": -1,
            "limit": 1
        }
    });
    let response = post("/api/mobile/corpus/changes", negative, Some(token)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let future = json!({
        "request": {
            "protocol_version": MOBILE_PROTOCOL_VERSION,
            "cursor": i64::MAX,
            "limit": 1
        }
    });
    let response = post("/api/mobile/cards/changes", future, Some(token)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = post(
        "/api/mobile/corpus/snapshot",
        snapshot_request(1, None, 1),
        Some(token),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

async fn assert_device_name_validation(token: &str) {
    for display_name in [String::from("   "), "界".repeat(129)] {
        let response = post(
            "/api/mobile/devices/register",
            json!({
                "request": {
                    "protocol_version": MOBILE_PROTOCOL_VERSION,
                    "device_id": Uuid::new_v4(),
                    "display_name": display_name
                }
            }),
            Some(token),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

async fn first_snapshot_page(token: &str) -> CorpusPageDto {
    let response = post(
        "/api/mobile/corpus/snapshot",
        snapshot_request(0, None, 2),
        Some(token),
    )
    .await;
    let response = assert_status(response, StatusCode::OK, "first snapshot page").await;
    let page: CorpusPageDto = response_json(response, "first snapshot page").await;
    assert_eq!(page.translations.len(), 2);
    assert!(page.has_more);
    page
}

async fn mutate_corpus_between_pages(db: &PgPool, fixture: &Fixture) {
    sqlx::query("UPDATE translations SET to_phrase = 'eins-neu', frequency = 9 WHERE id = $1")
        .bind(fixture.first)
        .execute(db)
        .await
        .expect("translation update");
    sqlx::query("DELETE FROM translations WHERE id = $1")
        .bind(fixture.second)
        .execute(db)
        .await
        .expect("translation delete");
    promote_phrase(db, fixture.de, fixture.en, fixture.third).await;
}

async fn second_snapshot_page(token: &str, first: &CorpusPageDto) -> CorpusPageDto {
    let response = post(
        "/api/mobile/corpus/snapshot",
        snapshot_request(first.next_cursor, Some(first.snapshot_watermark), 2),
        Some(token),
    )
    .await;
    let page: CorpusPageDto = response_json(response, "second snapshot page").await;
    assert_eq!(page.snapshot_watermark, first.snapshot_watermark);
    assert_eq!(page.translations.len(), 1);
    assert!(page.translations[0].is_phrase);
    assert!(!page.has_more);
    page
}

async fn demote_phrase(db: &PgPool, translation_id: i32) {
    sqlx::query("DELETE FROM phrase_translations WHERE translation_id = $1")
        .bind(translation_id)
        .execute(db)
        .await
        .expect("phrase demotion");
}

async fn interleaved_snapshot(db: &PgPool, fixture: &Fixture) -> (CorpusPageDto, CorpusPageDto) {
    let first = first_snapshot_page(&fixture.token).await;
    mutate_corpus_between_pages(db, fixture).await;
    let second = second_snapshot_page(&fixture.token, &first).await;
    demote_phrase(db, fixture.third).await;
    (first, second)
}

async fn collect_changes(
    token: &str,
    mut cursor: i64,
    state: &mut BTreeMap<i32, CorpusTranslationDto>,
) -> Vec<wisecrow_dto::CorpusChangeDto> {
    let mut observed = Vec::new();
    loop {
        let response = post(
            "/api/mobile/corpus/changes",
            corpus_change_request("en", "de", cursor, 2),
            Some(token),
        )
        .await;
        let page: CorpusChangePageDto = response_json(response, "corpus changes").await;
        assert!(page.next_cursor >= cursor);
        page.changes
            .iter()
            .for_each(|change| apply_corpus_change(state, change));
        observed.extend(page.changes);
        cursor = page.next_cursor;
        if !page.has_more {
            assert_eq!(cursor, page.change_watermark);
            return observed;
        }
    }
}

fn assert_phrase_and_delete_changes(changes: &[wisecrow_dto::CorpusChangeDto], fixture: &Fixture) {
    assert!(changes.iter().any(|change| {
        change.translation_id == fixture.second && change.kind == CorpusChangeKindDto::Delete
    }));
    assert!(changes.iter().any(|change| {
        change.translation_id == fixture.third
            && change
                .translation
                .as_ref()
                .is_some_and(|translation| translation.is_phrase)
    }));
    assert!(changes.iter().any(|change| {
        change.translation_id == fixture.third
            && change
                .translation
                .as_ref()
                .is_some_and(|translation| !translation.is_phrase)
    }));
}

async fn assert_corpus_convergence(db: &PgPool, fixture: &Fixture) {
    let (first, second) = interleaved_snapshot(db, fixture).await;
    let watermark = first.snapshot_watermark;
    let mut local: BTreeMap<_, _> = first
        .translations
        .into_iter()
        .chain(second.translations)
        .map(|translation| (translation.translation_id, translation))
        .collect();
    let changes = collect_changes(&fixture.token, watermark, &mut local).await;
    assert_phrase_and_delete_changes(&changes, fixture);
    assert_eq!(local, current_corpus(db).await);
}

async fn assert_wrong_pair_isolation(fixture: &Fixture) {
    let response = post(
        "/api/mobile/corpus/changes",
        corpus_change_request("en", "fr", 0, 500),
        Some(&fixture.token),
    )
    .await;
    let page: CorpusChangePageDto = response_json(response, "wrong-pair changes").await;
    assert!(page
        .changes
        .iter()
        .any(|change| change.translation_id == fixture.wrong_pair));
    assert!(page.changes.iter().all(|change| {
        ![fixture.first, fixture.second, fixture.third].contains(&change.translation_id)
    }));
}

async fn initial_card_cursor(db: &PgPool, fixture: &Fixture) -> i64 {
    seed_card(db, fixture.user_id, fixture.first).await;
    seed_card(db, fixture.user_id, fixture.third).await;
    seed_card(db, fixture.other_user_id, fixture.wrong_pair).await;
    let response = post(
        "/api/mobile/cards/changes",
        card_change_request(0),
        Some(&fixture.token),
    )
    .await;
    let page: CardChangePageDto = response_json(response, "initial cards").await;
    assert_eq!(page.changes.len(), 2);
    assert!(page.changes.iter().all(|change| match change {
        CardChangeDto::Upsert { card, .. } => card.translation_id != fixture.wrong_pair,
        CardChangeDto::Delete { translation_id, .. } => *translation_id != fixture.wrong_pair,
    }));
    page.next_cursor
}

async fn mutate_cards(db: &PgPool, fixture: &Fixture) {
    for reps in [3, 7] {
        sqlx::query("UPDATE cards SET reps = $1 WHERE user_id = $2 AND translation_id = $3")
            .bind(reps)
            .bind(fixture.user_id)
            .bind(fixture.first)
            .execute(db)
            .await
            .expect("card update");
    }
    sqlx::query("DELETE FROM cards WHERE user_id = $1 AND translation_id = $2")
        .bind(fixture.user_id)
        .bind(fixture.third)
        .execute(db)
        .await
        .expect("card delete");
}

async fn assert_changed_cards(fixture: &Fixture, cursor: i64) {
    let response = post(
        "/api/mobile/cards/changes",
        card_change_request(cursor),
        Some(&fixture.token),
    )
    .await;
    let page: CardChangePageDto = response_json(response, "changed cards").await;
    assert_eq!(page.changes.len(), 2);
    assert!(page.changes.iter().any(|change| matches!(
        change,
        CardChangeDto::Upsert { card, .. }
            if card.translation_id == fixture.first && card.reps == 7
    )));
    assert!(page.changes.iter().any(|change| matches!(
        change,
        CardChangeDto::Delete { translation_id, .. } if *translation_id == fixture.third
    )));
}

async fn assert_other_user_cards(fixture: &Fixture) {
    let response = post(
        "/api/mobile/cards/changes",
        card_change_request(0),
        Some(&fixture.other_token),
    )
    .await;
    let page: CardChangePageDto = response_json(response, "other cards").await;
    assert!(page.changes.iter().any(|change| match change {
        CardChangeDto::Upsert { card, .. } => card.translation_id == fixture.wrong_pair,
        CardChangeDto::Delete { translation_id, .. } => *translation_id == fixture.wrong_pair,
    }));
}

async fn assert_card_sync(db: &PgPool, fixture: &Fixture) {
    let cursor = initial_card_cursor(db, fixture).await;
    mutate_cards(db, fixture).await;
    assert_changed_cards(fixture, cursor).await;
    assert_other_user_cards(fixture).await;
}

async fn assert_revoked_device(db: &PgPool, fixture: &Fixture, device_id: Uuid) {
    sqlx::query(
        "UPDATE mobile_devices SET revoked_at = CURRENT_TIMESTAMP
         WHERE user_id = $1 AND id = $2",
    )
    .bind(fixture.user_id)
    .bind(device_id)
    .execute(db)
    .await
    .expect("device revoke");
    let response = post(
        "/api/mobile/devices/register",
        json!({
            "request": {
                "protocol_version": MOBILE_PROTOCOL_VERSION,
                "device_id": device_id,
                "display_name": "Cannot reactivate"
            }
        }),
        Some(&fixture.token),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn snapshot_changes_devices_and_cards_are_resumable_and_isolated() {
    std::env::set_var(
        "WISECROW__DB_URL",
        std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".into()),
    );
    init_pool().await.expect("pool");
    let db = pool().expect("pool");
    let fixture = setup_fixture(db).await;
    assert_capabilities().await;
    let device_id = register_device(&fixture.token).await;
    assert_protocol_and_cursor_validation(&fixture.token).await;
    assert_device_name_validation(&fixture.token).await;
    assert_corpus_convergence(db, &fixture).await;
    assert_wrong_pair_isolation(&fixture).await;
    assert_card_sync(db, &fixture).await;
    assert_revoked_device(db, &fixture, device_id).await;
    cleanup(db).await;
}
