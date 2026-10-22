//! First-page covers for books on the reader shelf.
//!
//! `GET /api/reader/covers/{id}` needs the session cookie. The JPEG is the
//! PDF's first page, cached under the media directory and keyed by the stored
//! file hash, so a replaced PDF is drawn again.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use axum::body::Body;
use axum::extract::{Extension, Path as BookId};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use super::auth::AuthenticatedSession;

const SHELF_DEFAULT: &str = "/app/readers";
const RENDER_LIMIT: Duration = Duration::from_secs(20);
const MAX_COVER_BYTES: u64 = 2 * 1024 * 1024;
const MAX_STDERR_BYTES: u64 = 4 * 1024;
static COVER_RENDERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

/// Cover routes merged into the fullstack router.
pub(crate) fn cover_routes() -> Router {
    Router::new().route("/api/reader/covers/{id}", get(book_cover))
}

async fn book_cover(
    session: Option<Extension<AuthenticatedSession>>,
    BookId(book_id): BookId<i32>,
) -> Response {
    if session.is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match cover_bytes(book_id).await {
        Ok(bytes) => jpeg_response(bytes),
        Err(failure) => failure.status().into_response(),
    }
}

fn jpeg_response(bytes: Vec<u8>) -> Response {
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=3600"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn cover_bytes(book_id: i32) -> Result<Vec<u8>, CoverFailure> {
    if book_id < 1 {
        return Err(CoverFailure::NotFound);
    }
    let row = cover_row(book_id).await?;
    let pdf = match contained_pdf(&reader_shelf(), &row.file) {
        Ok(pdf) => pdf,
        Err(ShelfPathError::Io(error)) => {
            tracing::warn!(book_id, ?error, "book file could not be resolved");
            return Err(CoverFailure::Failed);
        }
        Err(ShelfPathError::Missing | ShelfPathError::Rejected) => {
            tracing::warn!(book_id, "book file is not inside the reader shelf");
            return Err(CoverFailure::NotFound);
        }
    };
    let stem = cache_stem(book_id, &row.file_sha256)?;
    let cache = cover_cache_dir();
    if let Some(bytes) = cached_jpeg(&cache, &stem).await? {
        return Ok(bytes);
    }
    let _permit = COVER_RENDERS.acquire().await.map_err(failed)?;
    if let Some(bytes) = cached_jpeg(&cache, &stem).await? {
        return Ok(bytes);
    }
    render_and_store(&pdf, &cache, &stem).await
}

#[derive(sqlx::FromRow)]
struct CoverRow {
    file: String,
    file_sha256: Vec<u8>,
}

async fn cover_row(book_id: i32) -> Result<CoverRow, CoverFailure> {
    let db = super::pool().map_err(failed)?;
    sqlx::query_as::<_, CoverRow>(
        "SELECT file, file_sha256 FROM books WHERE id = $1 AND status = 'ready'",
    )
    .bind(book_id)
    .fetch_optional(db)
    .await
    .map_err(failed)?
    .ok_or(CoverFailure::NotFound)
}

fn reader_shelf() -> PathBuf {
    std::env::var_os("WISECROW__READER_SHELF")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(SHELF_DEFAULT))
}

fn cover_cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("WISECROW__COVER_CACHE").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    data_home().join("wisecrow").join("covers")
}

fn data_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) {
        return PathBuf::from(home).join(".local").join("share");
    }
    std::env::temp_dir()
}

#[derive(Debug)]
enum ShelfPathError {
    Rejected,
    Missing,
    Io(std::io::Error),
}

fn contained_pdf(root: &Path, file: &str) -> Result<PathBuf, ShelfPathError> {
    let relative = shelf_relative(file).ok_or(ShelfPathError::Rejected)?;
    let root = root.canonicalize().map_err(ShelfPathError::Io)?;
    let canonical = root.join(relative).canonicalize().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ShelfPathError::Missing
        } else {
            ShelfPathError::Io(error)
        }
    })?;
    if !canonical.starts_with(&root) || canonical == root || !canonical.is_file() {
        return Err(ShelfPathError::Rejected);
    }
    Ok(canonical)
}

fn shelf_relative(file: &str) -> Option<&Path> {
    if file.is_empty() || file.chars().any(char::is_control) {
        return None;
    }
    let path = Path::new(file);
    if path.is_absolute() {
        return None;
    }
    let mut saw_normal = false;
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let text = part.to_str()?;
                if text.contains('\\') {
                    return None;
                }
                saw_normal = true;
            }
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return None;
            }
        }
    }
    saw_normal.then_some(path)
}

fn cache_stem(book_id: i32, sha256: &[u8]) -> Result<String, CoverFailure> {
    if book_id < 1 || sha256.len() != 32 {
        return Err(CoverFailure::Failed);
    }
    let mut stem = String::with_capacity(3 + 1 + 64);
    stem.push_str(&book_id.to_string());
    stem.push('-');
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in sha256.iter().copied() {
        stem.push(char::from(HEX[usize::from(byte >> 4)]));
        stem.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(stem)
}

fn valid_stem(stem: &str) -> bool {
    let Some((id, sha)) = stem.split_once('-') else {
        return false;
    };
    !id.is_empty()
        && id.bytes().all(|byte| byte.is_ascii_digit())
        && sha.len() == 64
        && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
}

async fn cached_jpeg(cache: &Path, stem: &str) -> Result<Option<Vec<u8>>, CoverFailure> {
    if !valid_stem(stem) {
        return Err(CoverFailure::Failed);
    }
    let root = match tokio::fs::canonicalize(cache).await {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failed(error)),
    };
    let path = root.join(format!("{stem}.jpg"));
    let canonical = match tokio::fs::canonicalize(&path).await {
        Ok(canonical) => canonical,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failed(error)),
    };
    if !canonical.starts_with(&root) || !canonical.is_file() {
        tracing::warn!("cover cache file left the cache directory");
        return Err(CoverFailure::Failed);
    }
    read_cached_jpeg(&canonical).await
}

async fn read_cached_jpeg(path: &Path) -> Result<Option<Vec<u8>>, CoverFailure> {
    let meta = tokio::fs::metadata(path).await.map_err(failed)?;
    if !meta.is_file() || meta.len() < 3 || meta.len() > MAX_COVER_BYTES {
        remove_cache_file(path).await?;
        return Ok(None);
    }
    let bytes = tokio::fs::read(path).await.map_err(failed)?;
    if is_jpeg(&bytes) {
        return Ok(Some(bytes));
    }
    remove_cache_file(path).await?;
    Ok(None)
}

async fn remove_cache_file(path: &Path) -> Result<(), CoverFailure> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(failed(error)),
    }
}

async fn render_and_store(pdf: &Path, cache: &Path, stem: &str) -> Result<Vec<u8>, CoverFailure> {
    if !valid_stem(stem) {
        return Err(CoverFailure::Failed);
    }
    tokio::fs::create_dir_all(cache).await.map_err(failed)?;
    let root = tokio::fs::canonicalize(cache).await.map_err(failed)?;
    if !root.is_dir() {
        return Err(CoverFailure::Failed);
    }
    let final_path = root.join(format!("{stem}.jpg"));
    if final_path.parent() != Some(root.as_path()) {
        return Err(CoverFailure::Failed);
    }
    let nonce = uuid::Uuid::new_v4().simple();
    let file_stem = format!("{stem}-{nonce}");
    let prefix = root.join(&file_stem);
    if prefix.parent() != Some(root.as_path()) {
        return Err(CoverFailure::Failed);
    }
    let temp = TempJpeg(root.join(format!("{file_stem}.jpg")));
    render_first_page(pdf, &prefix).await?;
    let bytes = tokio::fs::read(&temp.0).await.map_err(failed)?;
    if !is_jpeg(&bytes) || !within_cover_limit(bytes.len()) {
        return Err(CoverFailure::Failed);
    }
    tokio::fs::rename(&temp.0, &final_path)
        .await
        .map_err(failed)?;
    Ok(bytes)
}

async fn render_first_page(pdf: &Path, prefix: &Path) -> Result<(), CoverFailure> {
    let mut child = Command::new("pdftoppm")
        .arg("-jpeg")
        .arg("-f")
        .arg("1")
        .arg("-l")
        .arg("1")
        .arg("-scale-to")
        .arg("480")
        .arg("-singlefile")
        .arg(pdf)
        .arg(prefix)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                tracing::error!("pdftoppm is not installed");
                CoverFailure::Failed
            } else {
                failed(error)
            }
        })?;
    let stderr = child.stderr.take();
    let logged = stderr.map(|pipe| {
        tokio::spawn(async move {
            let mut limited = pipe.take(MAX_STDERR_BYTES);
            let mut buf = Vec::new();
            if let Err(error) = limited.read_to_end(&mut buf).await {
                tracing::warn!(?error, "cover renderer stderr was not read");
            }
            buf
        })
    });
    let finished = tokio::time::timeout(RENDER_LIMIT, child.wait()).await;
    let status = match finished {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            drop(child);
            return Err(failed(error));
        }
        Err(error) => {
            drop(child);
            return Err(failed(error));
        }
    };
    if let Some(logged) = logged {
        match logged.await {
            Ok(buf) if !status.success() && !buf.is_empty() => {
                tracing::warn!(
                    stderr = %String::from_utf8_lossy(&buf),
                    "pdftoppm did not render a cover"
                );
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(?error, "cover stderr task failed"),
        }
    }
    if status.success() {
        Ok(())
    } else {
        Err(CoverFailure::Failed)
    }
}

struct TempJpeg(PathBuf);

impl Drop for TempJpeg {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.0) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(?error, "cover temp file was left behind"),
        }
    }
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
}

fn within_cover_limit(len: usize) -> bool {
    u64::try_from(len).is_ok_and(|len| len <= MAX_COVER_BYTES)
}

#[derive(Debug)]
enum CoverFailure {
    NotFound,
    Failed,
}

impl CoverFailure {
    fn status(self) -> StatusCode {
        match self {
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Failed => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

fn failed(error: impl std::fmt::Debug) -> CoverFailure {
    tracing::warn!(?error, "cover request failed");
    CoverFailure::Failed
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn write_pdf(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("directory");
        }
        std::fs::write(path, b"%PDF-1.4").expect("pdf");
    }

    #[test]
    fn a_book_file_must_stay_inside_the_shelf() {
        let root = tempfile::tempdir().expect("shelf");
        write_pdf(&root.path().join("Penguin Readers/Level 2/Book.pdf"));
        assert!(contained_pdf(root.path(), "Penguin Readers/Level 2/Book.pdf").is_ok());

        for escaped in [
            "../Book.pdf",
            "/etc/passwd",
            "Penguin Readers/../../Book.pdf",
            "",
            ".",
            "..",
            "Penguin Readers/Level 2/Book.pdf\n",
            "Penguin Readers\\Level 2\\Book.pdf",
        ] {
            assert!(contained_pdf(root.path(), escaped).is_err(), "{escaped}");
        }
    }

    #[test]
    fn a_link_that_leaves_the_shelf_is_not_a_cover_source() {
        let root = tempfile::tempdir().expect("shelf");
        let outside = tempfile::tempdir().expect("outside");
        let secret = outside.path().join("secret.pdf");
        write_pdf(&secret);
        write_pdf(&root.path().join("inside.pdf"));
        std::os::unix::fs::symlink(&secret, root.path().join("leak.pdf")).expect("symlink");
        std::os::unix::fs::symlink(
            root.path().join("inside.pdf"),
            root.path().join("alias.pdf"),
        )
        .expect("alias");

        assert!(contained_pdf(root.path(), "leak.pdf").is_err());
        assert!(contained_pdf(root.path(), "alias.pdf").is_ok());
    }

    #[test]
    fn a_directory_is_not_a_cover_source() {
        let root = tempfile::tempdir().expect("shelf");
        std::fs::create_dir(root.path().join("notes")).expect("directory");
        assert!(contained_pdf(root.path(), "notes").is_err());
    }

    #[test]
    fn a_cache_stem_is_the_book_and_the_file_hash() {
        let sha = [0xAB; 32];
        let stem = cache_stem(12, &sha).expect("stem");
        assert_eq!(stem, format!("12-{}", "ab".repeat(32)));
        assert!(valid_stem(&stem));
        assert!(cache_stem(0, &sha).is_err());
        assert!(cache_stem(12, &[0xAB; 31]).is_err());
        assert!(!valid_stem("../12"));
        assert!(!valid_stem("12-ab"));
    }

    #[tokio::test]
    async fn a_cached_cover_that_points_outside_the_cache_is_refused() {
        let cache = tempfile::tempdir().expect("cache");
        let outside = tempfile::tempdir().expect("outside");
        let stem = cache_stem(1, &[0x11; 32]).expect("stem");
        let secret = outside.path().join("secret.jpg");
        std::fs::write(&secret, [0xFF, 0xD8, 0xFF, 0x00]).expect("jpeg");
        std::os::unix::fs::symlink(&secret, cache.path().join(format!("{stem}.jpg")))
            .expect("symlink");

        assert!(cached_jpeg(cache.path(), &stem).await.is_err());
    }

    #[tokio::test]
    async fn a_cover_without_a_session_is_refused() {
        let response = cover_routes()
            .oneshot(
                Request::get("/api/reader/covers/1")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_merged_router_refuses_a_cover_without_a_session() {
        let public = tempfile::tempdir().expect("public assets");
        std::env::set_var("DIOXUS_PUBLIC_PATH", public.path());
        let response = super::super::build_router()
            .oneshot(
                Request::get("/api/reader/covers/4")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_first_page_is_stored_as_a_jpeg() {
        let cache = tempfile::tempdir().expect("cache");
        let pdf = cache.path().join("Book.pdf");
        std::fs::write(&pdf, one_page_pdf()).expect("pdf");
        let stem = cache_stem(3, &[0x22; 32]).expect("stem");

        let bytes = render_and_store(&pdf, cache.path(), &stem)
            .await
            .unwrap_or_else(|error| panic!("cover: {error:?}"));

        assert!(is_jpeg(&bytes), "pdftoppm did not write a jpeg");
        let cached = cached_jpeg(cache.path(), &stem)
            .await
            .unwrap_or_else(|error| panic!("cached cover: {error:?}"));
        assert_eq!(cached.as_deref(), Some(bytes.as_slice()));
    }

    fn one_page_pdf() -> Vec<u8> {
        let stream = "BT /F1 24 Tf 40 150 Td (Cover) Tj ET";
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        ];
        let mut out = Vec::from("%PDF-1.4\n");
        let mut offsets = Vec::with_capacity(objects.len());
        for (index, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            let number = index.saturating_add(1);
            out.extend(format!("{number} 0 obj\n{body}\nendobj\n").into_bytes());
        }
        let xref = out.len();
        let size = objects.len().saturating_add(1);
        out.extend(format!("xref\n0 {size}\n").into_bytes());
        out.extend(b"0000000000 65535 f \n");
        for offset in offsets {
            out.extend(format!("{offset:010} 00000 n \n").into_bytes());
        }
        out.extend(
            format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n")
                .into_bytes(),
        );
        out
    }

    #[test]
    fn jpeg_magic_rejects_a_short_or_other_file() {
        assert!(is_jpeg(&[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(!is_jpeg(&[0xFF, 0xD8]));
        assert!(!is_jpeg(b"%PDF-1.4"));
        assert!(!is_jpeg(&[]));
    }
}
