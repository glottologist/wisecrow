//! Request-level authorisation gate test. The `auth_enrich_layer` middleware is
//! what feeds `current_user`, which every protected server function calls before
//! trusting a request. This drives that layer end-to-end over HTTP: a valid
//! session cookie surfaces the `AuthUser`, and its absence or invalidity leaves
//! the request unauthenticated, so a protected handler rejects it.
//!
//! Complements `ownership.rs` (which proves cross-user rejection at the repository
//! layer) by proving the front-door gate itself.
//!
//! ```sh
//! TEST_DATABASE_URL=postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test \
//!   cargo test -p wisecrow-web --features server --test authz_gate -- --ignored
//! ```
#![cfg(feature = "server")]

use axum::extract::Extension;
use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;

use wisecrow::auth::hash_password;
use wisecrow_web::server::auth::{auth_enrich_layer, issue_session, AuthUser};
use wisecrow_web::server::{init_pool, pool};

/// Stand-in for a protected server function: 200 only when the middleware
/// attached an authenticated user, 401 otherwise — mirroring `current_user`.
async fn protected_probe(user: Option<Extension<AuthUser>>) -> StatusCode {
    match user {
        Some(Extension(_)) => StatusCode::OK,
        None => StatusCode::UNAUTHORIZED,
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn session_cookie_gates_protected_requests() {
    std::env::set_var(
        "WISECROW__DB_URL",
        std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".into()),
    );
    init_pool().await.expect("init pool");
    let db = pool().expect("pool");

    let email = "authz-gate@test.local";
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(email)
        .execute(db)
        .await
        .expect("cleanup");
    let hash = hash_password("hunter2").expect("hash");
    let uid: i32 = sqlx::query_scalar(
        "INSERT INTO users (display_name, email, password_hash, is_admin)
         VALUES ('Gate', $1, $2, false) RETURNING id",
    )
    .bind(email)
    .bind(&hash)
    .fetch_one(db)
    .await
    .expect("insert user");
    let token = issue_session(db, uid).await.expect("issue session");

    let app = Router::new()
        .route("/probe", get(protected_probe))
        .layer(axum::middleware::from_fn(auth_enrich_layer));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let http = reqwest::Client::new();
    let url = format!("http://{addr}/probe");
    let status = |cookie: Option<String>| {
        let http = http.clone();
        let url = url.clone();
        async move {
            let mut req = http.get(&url);
            if let Some(c) = cookie {
                req = req.header("cookie", format!("wisecrow_session={c}"));
            }
            req.send().await.expect("send").status().as_u16()
        }
    };

    assert_eq!(
        status(Some(token.clone())).await,
        200,
        "valid session cookie"
    );
    assert_eq!(status(None).await, 401, "no cookie");
    assert_eq!(
        status(Some("not-a-real-token".into())).await,
        401,
        "bad token"
    );

    server.abort();
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(email)
        .execute(db)
        .await
        .expect("cleanup");
}
