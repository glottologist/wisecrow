#![cfg(feature = "server")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

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
const MEDIA_ROUTES: &[(&str, &str)] = &[
    (
        "/api/media/audio",
        r#"{"translation_id":1,"foreign_phrase":"Hallo","foreign_lang":"de"}"#,
    ),
    ("/api/media/image", r#"{"translation_id":1,"word":"Hallo"}"#),
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

#[tokio::test]
async fn stable_protected_routes_are_registered() {
    let routes = AUTH_ROUTES
        .iter()
        .chain(LEARN_ROUTES)
        .chain(NBACK_ROUTES)
        .chain(QUIZ_ROUTES)
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
