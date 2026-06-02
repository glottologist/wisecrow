//! DB-backed integration test for the authenticated corpus-sync GET routes.
#![cfg(feature = "server")]

use wisecrow::sync::clients::SyncClientRepository;
use wisecrow_web::server::sync::sync_routes;
use wisecrow_web::server::{init_pool, pool};

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn sync_get_routes_authenticate() {
    std::env::set_var(
        "WISECROW__DB_URL",
        "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test",
    );
    std::env::set_var("WISECROW__SYNC_API_KEY", "legacy-fallback-key");
    init_pool().await.expect("init pool");
    let db = pool().expect("pool");

    sqlx::query("DELETE FROM sync_clients WHERE name = $1")
        .bind("itest")
        .execute(db)
        .await
        .expect("cleanup");
    let client_key = SyncClientRepository::add(db, "itest")
        .await
        .expect("add client");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, sync_routes()).await.expect("serve");
    });

    let http = reqwest::Client::new();
    let url = format!("http://{addr}/api/sync_languages?after_id=0");
    let status = |key: Option<&str>| {
        let http = http.clone();
        let url = url.clone();
        let key = key.map(str::to_owned);
        async move {
            let mut req = http.get(&url);
            if let Some(k) = key {
                req = req.header("x-api-key", k);
            }
            req.send().await.expect("send").status().as_u16()
        }
    };

    assert_eq!(status(Some(&client_key)).await, 200, "per-client key");
    assert_eq!(
        status(Some("legacy-fallback-key")).await,
        200,
        "fallback key"
    );
    assert_eq!(status(Some("nope")).await, 401, "wrong key");
    assert_eq!(status(None).await, 401, "missing key");

    server.abort();
    sqlx::query("DELETE FROM sync_clients WHERE name = $1")
        .bind("itest")
        .execute(db)
        .await
        .expect("cleanup");
}
