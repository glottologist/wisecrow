#![cfg(feature = "server")]

use axum::body::Body;
use axum::extract::Extension;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use tokio::sync::OnceCell;
use tower::ServiceExt;

use wisecrow::auth::hash_password;
use wisecrow_web::server::auth::{auth_enrich_layer, issue_session, AuthenticatedSession};
use wisecrow_web::server::{init_pool, pool};

const EMAIL: &str = "authz-gate@test.local";
const SIZE_EMAIL: &str = "deck-size@test.local";
const INVALID_TOKEN: &str = "not-a-real-token";
static TEST_INIT: OnceCell<()> = OnceCell::const_new();

async fn protected_probe(user: Option<Extension<AuthenticatedSession>>) -> StatusCode {
    match user {
        Some(Extension(_)) => StatusCode::OK,
        None => StatusCode::UNAUTHORIZED,
    }
}

async fn probe_status(
    app: &Router,
    cookie: Option<&str>,
    authorization: Option<&str>,
) -> StatusCode {
    let mut request = Request::get("/probe");
    if let Some(token) = cookie {
        request = request.header("cookie", ["wisecrow_session=", token].concat());
    }
    if let Some(value) = authorization {
        request = request.header("authorization", value);
    }
    app.clone()
        .oneshot(request.body(Body::empty()).expect("request"))
        .await
        .expect("response")
        .status()
}

async fn initialize_test_pool() {
    TEST_INIT
        .get_or_init(|| async {
            std::env::set_var(
                "WISECROW__DB_URL",
                std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
                    "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".into()
                }),
            );
            init_pool().await.expect("init pool");
        })
        .await;
}

async fn create_test_session(email: &str) -> String {
    let db = pool().expect("pool");
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(email)
        .execute(db)
        .await
        .expect("cleanup");
    let hash = hash_password("hunter2").expect("hash");
    let user_id: i32 = sqlx::query_scalar(
        "INSERT INTO users (display_name, email, password_hash, is_admin)
         VALUES ('Gate', $1, $2, false) RETURNING id",
    )
    .bind(email)
    .bind(&hash)
    .fetch_one(db)
    .await
    .expect("insert user");
    issue_session(db, user_id).await.expect("issue session")
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn session_credentials_gate_protected_requests() {
    initialize_test_pool().await;
    let token = create_test_session(EMAIL).await;
    let app = Router::new()
        .route("/probe", get(protected_probe))
        .layer(axum::middleware::from_fn(auth_enrich_layer));
    let bearer = ["Bearer ", token.as_str()].concat();
    let invalid_bearer = ["Bearer ", INVALID_TOKEN].concat();

    assert_eq!(probe_status(&app, Some(&token), None).await, StatusCode::OK);
    assert_eq!(
        probe_status(&app, None, Some(&bearer)).await,
        StatusCode::OK
    );
    for authorization in [
        None,
        Some("Basic abc"),
        Some("Bearer"),
        Some(invalid_bearer.as_str()),
    ] {
        assert_eq!(
            probe_status(&app, None, authorization).await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        probe_status(&app, Some(INVALID_TOKEN), None).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        probe_status(&app, Some(&token), Some(&invalid_bearer)).await,
        StatusCode::UNAUTHORIZED
    );

    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(EMAIL)
        .execute(pool().expect("pool"))
        .await
        .expect("cleanup");
}

async fn learning_post_status(token: &str, path: &str, body: String) -> StatusCode {
    let request = Request::post(path)
        .header("content-type", "application/json")
        .header("cookie", ["wisecrow_session=", token].concat())
        .body(Body::from(body))
        .expect("request");
    wisecrow_web::server::build_router()
        .oneshot(request)
        .await
        .expect("response")
        .status()
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn normal_and_fast_deck_size_boundaries_return_expected_status() {
    initialize_test_pool().await;
    let token = create_test_session(SIZE_EMAIL).await;
    for (path, size, expected) in [
        ("/api/learn/session/create", 0, StatusCode::BAD_REQUEST),
        ("/api/learn/session/create", 1, StatusCode::OK),
        ("/api/learn/session/create", 500, StatusCode::OK),
        ("/api/learn/session/create", 501, StatusCode::BAD_REQUEST),
        ("/api/learn/fast-deck", 0, StatusCode::BAD_REQUEST),
        ("/api/learn/fast-deck", 1, StatusCode::OK),
        ("/api/learn/fast-deck", 500, StatusCode::OK),
        ("/api/learn/fast-deck", 501, StatusCode::BAD_REQUEST),
    ] {
        let body = if path.ends_with("fast-deck") {
            serde_json::json!({"native": "en", "foreign": "de", "size": size}).to_string()
        } else {
            serde_json::json!({
                "native": "en",
                "foreign": "de",
                "deck_size": size,
                "speed_ms": 1000
            })
            .to_string()
        };
        assert_eq!(learning_post_status(&token, path, body).await, expected);
    }
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(SIZE_EMAIL)
        .execute(pool().expect("pool"))
        .await
        .expect("cleanup");
}
