//! Verifies `serve_tls` terminates TLS end-to-end: a trivial axum router is
//! served over HTTPS with a generated self-signed cert, and a TLS client
//! connects and gets the response. (The full Dioxus router needs `dx bundle`
//! assets, so it cannot run under `cargo test`; the TLS plumbing is identical
//! regardless of the router.)
#![cfg(feature = "server")]

use std::net::SocketAddr;

use axum::routing::get;
use axum::Router;

use wisecrow_web::server::tls::serve_tls;

#[tokio::test]
async fn serves_https_over_tls() {
    // Self-signed cert/key written to a temp dir.
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
        .expect("generate self-signed cert");
    let dir = tempfile::tempdir().expect("tempdir");
    let cert_path = dir.path().join("cert.pem");
    let key_path = dir.path().join("key.pem");
    std::fs::write(&cert_path, certified.cert.pem()).expect("write cert");
    std::fs::write(&key_path, certified.key_pair.serialize_pem()).expect("write key");

    let router = Router::new().route("/healthz", get(|| async { "ok" }));
    let addr: SocketAddr = "127.0.0.1:19443".parse().unwrap();

    let server = tokio::spawn(async move {
        let _ = serve_tls(router, addr, &cert_path, &key_path).await;
    });

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("build client");

    let mut result = None;
    for _ in 0..40 {
        if let Ok(resp) = client.get("https://127.0.0.1:19443/healthz").send().await {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            result = Some((status, body));
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    server.abort();

    let (status, body) = result.expect("HTTPS server never became reachable");
    assert_eq!(status, 200, "expected 200 over TLS");
    assert_eq!(body, "ok");
}
