//! DB-backed integration tests for the web auth session store.
//!
//! Only meaningful with the `server` feature; the whole file compiles to nothing
//! otherwise. Run via a Postgres on `TEST_DATABASE_URL` (defaults to the
//! integration container on :5433):
//!
//! ```sh
//! TEST_DATABASE_URL=postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test \
//!   cargo test -p wisecrow-web --features server --test auth_flow -- --ignored
//! ```
#![cfg(feature = "server")]

use sqlx::PgPool;

use wisecrow::auth::hash_password;
use wisecrow_web::server::auth;

async fn test_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url)
        .await
        .expect("Failed to connect to test database");
    sqlx::migrate!("../wisecrow-core/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn session_issue_lookup_revoke() {
    let pool = test_pool().await;
    let email = "p1-auth-test@example.com";

    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(email)
        .execute(&pool)
        .await
        .expect("cleanup failed");

    let hash = hash_password("hunter2").expect("hash");
    let uid: i32 = sqlx::query_scalar(
        "INSERT INTO users (display_name, email, password_hash, is_admin)
         VALUES ('Tester', $1, $2, true) RETURNING id",
    )
    .bind(email)
    .bind(&hash)
    .fetch_one(&pool)
    .await
    .expect("insert user failed");

    // Issue → look up: the token resolves to the right user, with is_admin.
    let token = auth::issue_session(&pool, uid).await.expect("issue");
    let who = auth::user_for_token(&pool, &token)
        .await
        .expect("lookup")
        .expect("session should resolve to a user");
    assert_eq!(who.id, uid);
    assert!(who.is_admin);

    // An unknown token resolves to nothing.
    assert!(auth::user_for_token(&pool, "not-a-real-token")
        .await
        .expect("lookup unknown")
        .is_none());

    // Revoke → the token no longer resolves.
    auth::revoke_session(&pool, &token).await.expect("revoke");
    assert!(auth::user_for_token(&pool, &token)
        .await
        .expect("lookup after revoke")
        .is_none());

    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(email)
        .execute(&pool)
        .await
        .expect("cleanup failed");
}
