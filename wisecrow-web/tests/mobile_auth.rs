#![cfg(feature = "server")]

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use serde::de::DeserializeOwned;
use serde_json::json;
use tower::ServiceExt;
use wisecrow::auth::hash_password;
use wisecrow_dto::{MobileSessionDto, UserDto};
use wisecrow_web::server::{build_router, init_pool, pool};

const EMAIL: &str = "mobile-auth@test.local";
const PASSWORD: &str = "mobile-test-password";

async fn post(path: &str, body: serde_json::Value, bearer: Option<&str>) -> Response {
    let mut request = Request::post(path).header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = bearer {
        request = request.header(header::AUTHORIZATION, ["Bearer ", token].concat());
    }
    build_router()
        .oneshot(request.body(Body::from(body.to_string())).expect("request"))
        .await
        .expect("response")
}

async fn response_json<T: DeserializeOwned>(response: Response, label: &str) -> T {
    let body = to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect(label);
    serde_json::from_slice(&body).expect(label)
}

async fn create_user() {
    let db = pool().expect("pool");
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(EMAIL)
        .execute(db)
        .await
        .expect("cleanup");
    let hash = hash_password(PASSWORD).expect("hash");
    sqlx::query(
        "INSERT INTO users (display_name, email, password_hash, is_admin)
         VALUES ('Mobile', $1, $2, false)",
    )
    .bind(EMAIL)
    .bind(hash)
    .execute(db)
    .await
    .expect("user");
}

async fn delete_user() {
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(EMAIL)
        .execute(pool().expect("pool"))
        .await
        .expect("cleanup");
}

async fn assert_browser_cookie_issued() {
    let response = post(
        "/api/auth/login",
        json!({ "email": EMAIL, "password": PASSWORD }),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .expect("secure session cookie");
    assert!(cookie.starts_with("wisecrow_session="));
    assert!(cookie.contains("; HttpOnly; Secure; SameSite=Strict; Path=/;"));
}

async fn mobile_login() -> MobileSessionDto {
    let response = post(
        "/api/mobile/login",
        json!({ "email": EMAIL, "password": PASSWORD }),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    response_json(response, "login response").await
}

async fn assert_identity(token: &str, expected_user_id: i32) {
    let response = post("/api/mobile/me", json!({}), Some(token)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let user: UserDto = response_json(response, "identity response").await;
    assert_eq!(user.id, expected_user_id);
}

async fn assert_unauthorized(token: &str) {
    let response = post("/api/mobile/me", json!({}), Some(token)).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn native_session_lifecycle() {
    std::env::set_var(
        "WISECROW__DB_URL",
        std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".into()),
    );
    init_pool().await.expect("pool");
    create_user().await;

    let first = mobile_login().await;
    let second = mobile_login().await;
    assert_eq!(first.user.display_name, "Mobile");
    assert_ne!(first.token, second.token);
    assert_identity(&first.token, first.user.id).await;
    assert_identity(&second.token, second.user.id).await;

    let response = post("/api/mobile/logout", json!({}), Some(&first.token)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_unauthorized(&first.token).await;
    assert_identity(&second.token, second.user.id).await;

    assert_browser_cookie_issued().await;
    delete_user().await;
}
