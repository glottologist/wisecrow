//! DB-backed tests for the admin provisioning repository methods (the `user` and
//! `sync-client` CLI commands sit on top of these).

use sqlx::PgPool;

use wisecrow::auth::{hash_password, verify_password};
use wisecrow::sync::clients::SyncClientRepository;
use wisecrow::users::UserRepository;

async fn test_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url)
        .await
        .expect("Failed to connect to test database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn provision_set_password_disable() {
    let pool = test_pool().await;
    let email = "admin-prov@test.local";
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(email)
        .execute(&pool)
        .await
        .expect("cleanup");

    let hash = hash_password("secret1").unwrap();
    let user = UserRepository::create_with_credentials(&pool, email, "Admin", &hash, true)
        .await
        .expect("create");
    assert!(user.id > 0);

    let (stored, is_admin): (Option<String>, bool) =
        sqlx::query_as("SELECT password_hash, is_admin FROM users WHERE email = $1")
            .bind(email)
            .fetch_one(&pool)
            .await
            .expect("fetch");
    assert!(verify_password("secret1", &stored.expect("hash present")));
    assert!(is_admin);

    // set_password replaces the hash.
    let h2 = hash_password("secret2").unwrap();
    assert!(UserRepository::set_password(&pool, email, &h2)
        .await
        .unwrap());
    let s2: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users WHERE email = $1")
        .bind(email)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(verify_password("secret2", &s2.expect("hash present")));

    // disable clears the hash.
    assert!(UserRepository::disable(&pool, email).await.unwrap());
    let s3: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users WHERE email = $1")
        .bind(email)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(s3.is_none());

    // list_accounts surfaces the account.
    let accounts = UserRepository::list_accounts(&pool).await.unwrap();
    assert!(accounts
        .iter()
        .any(|(_, e, _, _)| e.as_deref() == Some(email)));

    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(email)
        .execute(&pool)
        .await
        .expect("cleanup");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn sync_client_add_verify_revoke() {
    let pool = test_pool().await;
    let name = "test-laptop";
    sqlx::query("DELETE FROM sync_clients WHERE name = $1")
        .bind(name)
        .execute(&pool)
        .await
        .expect("cleanup");

    let key = SyncClientRepository::add(&pool, name).await.expect("add");
    assert!(SyncClientRepository::verify(&pool, &key).await.unwrap());
    assert!(!SyncClientRepository::verify(&pool, "wrong-key")
        .await
        .unwrap());

    assert!(SyncClientRepository::revoke(&pool, name).await.unwrap());
    // A revoked key no longer verifies.
    assert!(!SyncClientRepository::verify(&pool, &key).await.unwrap());

    sqlx::query("DELETE FROM sync_clients WHERE name = $1")
        .bind(name)
        .execute(&pool)
        .await
        .expect("cleanup");
}
