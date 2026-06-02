//! App-terminated TLS: serve the fullstack router over HTTPS with axum-server +
//! rustls, loading the certificate and key from operator-managed paths. The
//! operator owns certificate renewal; a renewal is picked up on container
//! restart.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use axum::Router;
use axum_server::tls_rustls::RustlsConfig;

/// Reads the TLS cert/key paths from the environment. Returns `None` (serve plain
/// HTTP) when either variable is unset or empty.
#[must_use]
pub fn tls_paths_from_env() -> Option<(PathBuf, PathBuf)> {
    let cert = std::env::var("WISECROW__TLS_CERT_PATH").ok()?;
    let key = std::env::var("WISECROW__TLS_KEY_PATH").ok()?;
    if cert.is_empty() || key.is_empty() {
        return None;
    }
    Some((PathBuf::from(cert), PathBuf::from(key)))
}

/// Resolves the bind address from `IP`/`PORT` (defaulting to `0.0.0.0` and
/// `default_port`).
#[must_use]
pub fn bind_addr(default_port: u16) -> SocketAddr {
    let ip = std::env::var("IP").unwrap_or_else(|_| "0.0.0.0".to_owned());
    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(default_port);
    format!("{ip}:{port}")
        .parse()
        .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], default_port)))
}

/// Serves `router` over HTTPS on `addr`, terminating TLS with the PEM cert/key at
/// the given paths. Runs until the server stops.
///
/// # Errors
///
/// Returns an error if the cert/key cannot be loaded or the listener fails.
pub async fn serve_tls(
    router: Router,
    addr: SocketAddr,
    cert: &Path,
    key: &Path,
) -> std::io::Result<()> {
    // rustls 0.23 requires a process-default crypto provider; install aws-lc-rs
    // (rustls' default backend). Ignore the error if one is already installed.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let config = RustlsConfig::from_pem_file(cert, key).await?;
    tracing::info!("Serving HTTPS on {addr}");
    axum_server::bind_rustls(addr, config)
        .serve(router.into_make_service())
        .await
}
