#![cfg(feature = "server")]

use sqlx::PgPool;

use wisecrow::auth::{hash_password, hash_token};
use wisecrow_web::server::auth;

const EMAIL: &str = "p1-auth-test@example.com";

async fn test_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        String::from("postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test")
    });
    let pool = PgPool::connect(&url)
        .await
        .expect("Failed to connect to test database");
    sqlx::migrate!("../wisecrow-core/migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
}

async fn create_test_user(pool: &PgPool) -> i32 {
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(EMAIL)
        .execute(pool)
        .await
        .expect("cleanup failed");
    let hash = hash_password("hunter2").expect("hash");
    sqlx::query_scalar(
        "INSERT INTO users (display_name, email, password_hash, is_admin)
         VALUES ('Tester', $1, $2, true) RETURNING id",
    )
    .bind(EMAIL)
    .bind(&hash)
    .fetch_one(pool)
    .await
    .expect("insert user failed")
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn session_issue_lookup_revoke() {
    let pool = test_pool().await;
    let uid = create_test_user(&pool).await;

    let token = auth::issue_session(&pool, uid).await.expect("issue");
    let session = auth::user_session_for_token(&pool, &token)
        .await
        .expect("lookup")
        .expect("session should resolve to a user");
    assert_eq!(session.user.id, uid);
    assert!(session.user.is_admin);
    assert_eq!(session.token_hash(), hash_token(&token));

    assert!(auth::user_session_for_token(&pool, "not-a-real-token")
        .await
        .expect("lookup unknown")
        .is_none());

    auth::revoke_session_hash(&pool, session.token_hash())
        .await
        .expect("revoke");
    assert!(auth::user_session_for_token(&pool, &token)
        .await
        .expect("lookup after revoke")
        .is_none());

    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(EMAIL)
        .execute(&pool)
        .await
        .expect("cleanup failed");
}
