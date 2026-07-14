//! Web authentication: opaque server-side sessions, the `login`/`logout` server
//! functions, the `current_user` gate used by protected server functions, and an
//! enrichment middleware that attaches the authenticated user to each request.

use axum::extract::Request;
use axum::http::{header::SET_COOKIE, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum::Extension;
use axum_extra::extract::CookieJar;
use dioxus::fullstack::FullstackContext;
use dioxus::prelude::*;
use sqlx::PgPool;

use std::sync::OnceLock;

use wisecrow::auth::{generate_session_token, hash_password, hash_token, verify_password};
use wisecrow_dto::UserDto;

use super::pool;

const COOKIE_NAME: &str = "wisecrow_session";
const SESSION_DAYS: i32 = 30;
const SESSION_MAX_AGE_SECS: i32 = SESSION_DAYS * 24 * 60 * 60;

/// A stable Argon2 hash of a throwaway secret, verified against when the supplied
/// email has no account (or no password). Running the verification unconditionally
/// keeps `login`'s response time independent of whether the email exists, closing
/// the enumeration-by-timing oracle.
fn dummy_password_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| hash_password("wisecrow-timing-equaliser").unwrap_or_default())
}

/// The authenticated user for a request, injected into request extensions by
/// [`auth_enrich_layer`] and read back by [`current_user`].
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i32,
    pub is_admin: bool,
}

/// Issues a new session for `user_id`, returning the raw token (the caller sets
/// it as a cookie). Only the SHA-256 hash of the token is stored.
///
/// # Errors
///
/// Returns the underlying [`sqlx::Error`] if the insert fails.
pub async fn issue_session(db: &PgPool, user_id: i32) -> Result<String, sqlx::Error> {
    let token = generate_session_token();
    sqlx::query(
        "INSERT INTO auth_sessions (user_id, token_hash, expires_at)
         VALUES ($1, $2, now() + make_interval(days => $3))",
    )
    .bind(user_id)
    .bind(hash_token(&token))
    .bind(SESSION_DAYS)
    .execute(db)
    .await?;
    Ok(token)
}

/// Resolves the live (unexpired) session for `token` to its user, bumping
/// `last_used_at` on a hit. Returns `None` when the token is unknown or expired.
///
/// # Errors
///
/// Returns the underlying [`sqlx::Error`] if the lookup fails.
pub async fn user_for_token(db: &PgPool, token: &str) -> Result<Option<AuthUser>, sqlx::Error> {
    let hash = hash_token(token);
    let row = sqlx::query_as::<_, (i32, bool)>(
        "SELECT u.id, u.is_admin
         FROM auth_sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.expires_at > now()",
    )
    .bind(&hash)
    .fetch_optional(db)
    .await?;

    if row.is_some() {
        // Throttle the bookkeeping write: refresh `last_used_at` at most once every
        // few minutes per session so a valid cookie does not incur a write on every
        // request (static assets included).
        let _ = sqlx::query(
            "UPDATE auth_sessions SET last_used_at = now()
             WHERE token_hash = $1
               AND (last_used_at IS NULL OR last_used_at < now() - interval '5 minutes')",
        )
        .bind(&hash)
        .execute(db)
        .await;
    }
    Ok(row.map(|(id, is_admin)| AuthUser { id, is_admin }))
}

/// Deletes the session identified by `token`, if any.
///
/// # Errors
///
/// Returns the underlying [`sqlx::Error`] if the delete fails.
pub async fn revoke_session(db: &PgPool, token: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM auth_sessions WHERE token_hash = $1")
        .bind(hash_token(token))
        .execute(db)
        .await?;
    Ok(())
}

/// Returns the authenticated user for the current request, or a 401 error when
/// no valid session cookie was presented. Protected server functions call this
/// first and use the returned `id` instead of any client-supplied `user_id`.
///
/// # Errors
///
/// Returns a `ServerFnError` (after committing a 401 status) when the request
/// has no authenticated user.
pub async fn current_user() -> Result<AuthUser, ServerFnError> {
    match FullstackContext::extract::<Extension<AuthUser>, _>().await {
        Ok(Extension(user)) => Ok(user),
        Err(_) => {
            FullstackContext::commit_http_status(
                StatusCode::UNAUTHORIZED,
                Some("Unauthorized".into()),
            );
            Err(ServerFnError::new("Unauthorized"))
        }
    }
}

fn push_set_cookie(cookie: &str) -> Result<(), ServerFnError> {
    let value = HeaderValue::from_str(cookie)
        .map_err(|e| ServerFnError::new(format!("invalid cookie: {e}")))?;
    if let Some(ctx) = FullstackContext::current() {
        ctx.add_response_header(SET_COOKIE, value);
    }
    Ok(())
}

fn set_session_cookie(token: &str) -> Result<(), ServerFnError> {
    push_set_cookie(&format!(
        "{COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={SESSION_MAX_AGE_SECS}"
    ))
}

fn clear_session_cookie() -> Result<(), ServerFnError> {
    push_set_cookie(&format!(
        "{COOKIE_NAME}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0"
    ))
}

/// Authenticates an email/password, issues a session, and sets the session
/// cookie. Returns the logged-in user.
#[server]
pub async fn login(email: String, password: String) -> Result<UserDto, ServerFnError> {
    let db = pool()?;
    let row = sqlx::query_as::<_, (i32, String, Option<String>)>(
        "SELECT id, display_name, password_hash FROM users WHERE email = $1",
    )
    .bind(&email)
    .fetch_optional(db)
    .await
    .map_err(|e| ServerFnError::new(format!("login query failed: {e}")))?;

    // Verify a password on every path — against the stored hash when the account
    // exists, else a fixed dummy hash — so timing does not reveal whether the
    // email is registered.
    let (found, hash) = match row {
        Some((id, display_name, Some(hash))) => (Some((id, display_name)), hash),
        _ => (None, dummy_password_hash().to_owned()),
    };
    let password_ok = verify_password(&password, &hash);
    let Some((id, display_name)) = found.filter(|_| password_ok) else {
        return Err(ServerFnError::new("Invalid email or password"));
    };

    let token = issue_session(db, id)
        .await
        .map_err(|e| ServerFnError::new(format!("session creation failed: {e}")))?;
    set_session_cookie(&token)?;

    Ok(UserDto { id, display_name })
}

/// Revokes the current session and clears the cookie.
#[server]
pub async fn logout() -> Result<(), ServerFnError> {
    let jar = FullstackContext::extract::<CookieJar, _>()
        .await
        .map_err(|_| ServerFnError::new("no request context"))?;
    if let Some(cookie) = jar.get(COOKIE_NAME) {
        if let Ok(db) = pool() {
            let _ = revoke_session(db, cookie.value()).await;
        }
    }
    clear_session_cookie()?;
    Ok(())
}

/// Enrichment middleware: if the request carries a valid session cookie, attach
/// the [`AuthUser`] to the request extensions. Never rejects — the login page,
/// static assets, and sync routes must stay reachable; rejection is enforced
/// per-function by [`current_user`].
pub async fn auth_enrich_layer(jar: CookieJar, mut req: Request, next: Next) -> Response {
    if let Some(cookie) = jar.get(COOKIE_NAME) {
        if let Ok(db) = pool() {
            if let Ok(Some(user)) = user_for_token(db, cookie.value()).await {
                req.extensions_mut().insert(user);
            }
        }
    }
    next.run(req).await
}
