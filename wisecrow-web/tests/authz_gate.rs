#![cfg(feature = "server")]

use axum::body::Body;
use axum::extract::Extension;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use tower::ServiceExt;

use wisecrow::auth::hash_password;
use wisecrow_web::server::auth::{auth_enrich_layer, issue_session, AuthenticatedSession};
use wisecrow_web::server::{init_pool, pool};

const EMAIL: &str = "authz-gate@test.local";
const INVALID_TOKEN: &str = "not-a-real-token";

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

async fn create_test_session() -> String {
    let db = pool().expect("pool");
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(EMAIL)
        .execute(db)
        .await
        .expect("cleanup");
    let hash = hash_password("hunter2").expect("hash");
    let user_id: i32 = sqlx::query_scalar(
        "INSERT INTO users (display_name, email, password_hash, is_admin)
         VALUES ('Gate', $1, $2, false) RETURNING id",
    )
    .bind(EMAIL)
    .bind(&hash)
    .fetch_one(db)
    .await
    .expect("insert user");
    issue_session(db, user_id).await.expect("issue session")
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn session_credentials_gate_protected_requests() {
    std::env::set_var(
        "WISECROW__DB_URL",
        std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".into()),
    );
    init_pool().await.expect("init pool");
    let token = create_test_session().await;
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
