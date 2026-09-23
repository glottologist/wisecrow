#![cfg(feature = "server")]

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use wisecrow_dto::{
    MobileCapabilitiesDto, MobileFeatureDto, MOBILE_PROTOCOL_VERSION, MOBILE_PROTOCOL_VERSION_V2,
};

const AUTH_ROUTES: &[(&str, &str)] = &[("/api/mobile/me", "{}"), ("/api/mobile/logout", "{}")];
const PUBLIC_AUTH_ROUTES: &[(&str, &str)] = &[
    (
        "/api/auth/login",
        r#"{"email":"test@example.com","password":"invalid"}"#,
    ),
    ("/api/auth/logout", "{}"),
    (
        "/api/mobile/login",
        r#"{"email":"test@example.com","password":"invalid"}"#,
    ),
];
const LEARN_ROUTES: &[(&str, &str)] = &[
    ("/api/learn/languages", "{}"),
    (
        "/api/learn/session/create",
        r#"{"native":"en","foreign":"de","deck_size":10,"speed_ms":1000}"#,
    ),
    (
        "/api/learn/session/resume",
        r#"{"native":"en","foreign":"de"}"#,
    ),
    (
        "/api/learn/card/answer",
        r#"{"session_id":1,"card_id":1,"rating":"Good"}"#,
    ),
    ("/api/learn/session/pause", r#"{"session_id":1}"#),
    (
        "/api/learn/fast-deck",
        r#"{"native":"en","foreign":"de","size":100}"#,
    ),
    ("/api/learn/session/complete", r#"{"session_id":1}"#),
];
const NBACK_ROUTES: &[(&str, &str)] = &[
    (
        "/api/nback/start",
        r#"{"config":{"mode":"AudioWritten","n_level":2,"interval_ms":4000,"native_lang":"en","foreign_lang":"de"}}"#,
    ),
    (
        "/api/nback/trial",
        r#"{"session_id":1,"trial_result":{"trial_number":0,"audio_response":false,"visual_response":false,"response_time_ms":100}}"#,
    ),
    ("/api/nback/complete", r#"{"session_id":1}"#),
];
const QUIZ_ROUTES: &[(&str, &str)] = &[
    (
        "/api/quiz/pdf",
        r#"{"pdf_bytes":[37,80,68,70,45],"num_questions":1}"#,
    ),
    (
        "/api/quiz/rule",
        r#"{"lang":"de","level":"A1","num_questions":1}"#,
    ),
];
const GRAMMAR_ROUTES: &[(&str, &str)] = &[
    (
        "/api/grammar/session/start",
        r#"{"native":"en","foreign":"es","level":null}"#,
    ),
    (
        "/api/grammar/session/submit",
        r#"{"submission":{"session_id":"019131c0-7f68-7b31-a775-2d6f91aa3196","event_id":"019131c0-7f68-7b31-a775-2d6f91aa3197","item_id":1,"revision":1,"answer":"estoy","chose_option":false,"hint_shown":false,"ordinal":1,"occurred_at":"2026-09-23T00:00:00Z"}}"#,
    ),
    (
        "/api/grammar/session/complete",
        r#"{"session_id":"019131c0-7f68-7b31-a775-2d6f91aa3196"}"#,
    ),
    ("/api/grammar/brainmap", r#"{"native":"en","foreign":"es"}"#),
    (
        "/api/grammar/placement/start",
        r#"{"native":"en","foreign":"es"}"#,
    ),
    (
        "/api/grammar/placement/submit",
        r#"{"attempt_id":"019131c0-7f68-7b31-a775-2d6f91aa3196","answers":[]}"#,
    ),
];
const MEDIA_ROUTES: &[(&str, &str)] = &[
    ("/api/media/audio", r#"{"translation_id":1}"#),
    ("/api/media/image", r#"{"translation_id":1}"#),
];

async fn post_status(path: &str, body: &str) -> StatusCode {
    let request = Request::post(path)
        .header("content-type", "application/json")
        .body(Body::from(String::from(body)))
        .expect("request");
    wisecrow_web::server::build_router()
        .oneshot(request)
        .await
        .expect("response")
        .status()
}

async fn post_json<T: serde::de::DeserializeOwned>(path: &str, body: &str) -> T {
    let request = Request::post(path)
        .header("content-type", "application/json")
        .body(Body::from(String::from(body)))
        .expect("request");
    let response = wisecrow_web::server::build_router()
        .oneshot(request)
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK, "{path}");
    let bytes = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!("{path}: {error}; body: {}", String::from_utf8_lossy(&bytes))
    })
}

#[tokio::test]
async fn stable_protected_routes_are_registered() {
    let routes = AUTH_ROUTES
        .iter()
        .chain(LEARN_ROUTES)
        .chain(NBACK_ROUTES)
        .chain(QUIZ_ROUTES)
        .chain(GRAMMAR_ROUTES)
        .chain(MEDIA_ROUTES);
    for &(path, body) in routes {
        assert_eq!(
            post_status(path, body).await,
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
    }
}

#[tokio::test]
async fn stable_public_auth_routes_are_registered() {
    for &(path, body) in PUBLIC_AUTH_ROUTES {
        let status = post_status(path, body).await;
        assert!(
            !matches!(
                status,
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ),
            "{path}"
        );
    }
}

#[tokio::test]
async fn mobile_capabilities_route_is_public() {
    let status = post_status("/api/mobile/capabilities", "{}").await;
    assert!(
        !matches!(
            status,
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
        ),
        "/api/mobile/capabilities"
    );
}

/// Version 1 must keep answering 1 whatever version 2 says, because a deployed
/// client compares the number for equality and refuses anything else.
#[tokio::test]
async fn version_one_discovery_is_unchanged_by_version_two() {
    let v1: MobileCapabilitiesDto = post_json("/api/mobile/capabilities", "{}").await;
    assert_eq!(v1.protocol_version, MOBILE_PROTOCOL_VERSION);
    assert!(!v1.supported_features.iter().any(|feature| matches!(
        feature,
        MobileFeatureDto::GrammarBankSync
            | MobileFeatureDto::GrammarMasterySync
            | MobileFeatureDto::GrammarAttemptUpload
    )));

    let v2: MobileCapabilitiesDto = post_json("/api/mobile/v2/capabilities", "{}").await;
    assert_eq!(v2.protocol_version, MOBILE_PROTOCOL_VERSION_V2);
    for feature in [
        MobileFeatureDto::GrammarBankSync,
        MobileFeatureDto::GrammarMasterySync,
        MobileFeatureDto::GrammarAttemptUpload,
    ] {
        assert!(
            v2.supported_features.contains(&feature),
            "version 2 must advertise {feature:?}"
        );
    }
    assert!(
        v2.supported_features
            .contains(&MobileFeatureDto::CorpusSync),
        "version 2 keeps everything version 1 offered"
    );
}

#[rstest::rstest]
#[case(
    "/api/mobile/devices/register",
    r#"{"request":{"protocol_version":1,"device_id":"019131c0-7f68-7b31-a775-2d6f91aa3196","display_name":"Test phone"}}"#
)]
#[case(
    "/api/mobile/corpus/snapshot",
    r#"{"request":{"protocol_version":1,"pair":{"native_lang":"en","foreign_lang":"de"},"after_translation_id":0,"snapshot_watermark":null,"limit":100}}"#
)]
#[case(
    "/api/mobile/corpus/changes",
    r#"{"request":{"protocol_version":1,"pair":{"native_lang":"en","foreign_lang":"de"},"cursor":0,"limit":100}}"#
)]
#[case(
    "/api/mobile/cards/changes",
    r#"{"request":{"protocol_version":1,"cursor":0,"limit":100}}"#
)]
#[case(
    "/api/mobile/reviews/upload",
    r#"{"request":{"protocol_version":1,"device_id":"019131c0-7f68-7b31-a775-2d6f91aa3196","events":[]}}"#
)]
#[case(
    "/api/mobile/nback/upload",
    r#"{"request":{"protocol_version":1,"device_id":"019131c0-7f68-7b31-a775-2d6f91aa3196","sessions":[]}}"#
)]
#[tokio::test]
async fn mobile_sync_route_requires_authentication(#[case] path: &str, #[case] body: &str) {
    assert_eq!(
        post_status(path, body).await,
        StatusCode::UNAUTHORIZED,
        "{path}"
    );
}
