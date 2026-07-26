//! Web authentication with opaque server-side sessions.

use axum::extract::Request;
use axum::http::{
    header::{AUTHORIZATION, SET_COOKIE},
    HeaderValue, StatusCode,
};
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
const SESSION_TOKEN_MAX_LENGTH: usize = 43;
const EMAIL_MAX_LENGTH: usize = 255;
const PASSWORD_MAX_LENGTH: usize = 1024;

fn server_error(message: &str) -> ServerFnError {
    ServerFnError::ServerError {
        message: String::from(message),
        code: StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        details: None,
    }
}

fn unauthorized_error(message: &str) -> ServerFnError {
    ServerFnError::ServerError {
        message: String::from(message),
        code: StatusCode::UNAUTHORIZED.as_u16(),
        details: None,
    }
}

fn dummy_password_hash() -> Result<&'static str, ServerFnError> {
    static DUMMY: OnceLock<Result<String, ()>> = OnceLock::new();
    match DUMMY.get_or_init(|| {
        hash_password("wisecrow-timing-equaliser").map_err(|error| {
            tracing::error!(?error, "failed to initialize password timing equalizer");
        })
    }) {
        Ok(hash) => Ok(hash),
        Err(()) => Err(server_error("Authentication service unavailable")),
    }
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub id: i32,
    pub is_admin: bool,
}

#[derive(Clone)]
pub struct AuthenticatedSession {
    pub user: AuthUser,
    token_hash: Vec<u8>,
}

impl AuthenticatedSession {
    #[must_use]
    pub fn token_hash(&self) -> &[u8] {
        &self.token_hash
    }
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

/// Resolves a live session and bumps `last_used_at` on a hit.
///
/// # Errors
///
/// Returns the underlying [`sqlx::Error`] if the lookup fails.
pub async fn user_session_for_token(
    db: &PgPool,
    token: &str,
) -> Result<Option<AuthenticatedSession>, sqlx::Error> {
    let token_hash = hash_token(token);
    let row = sqlx::query_as::<_, (i32, bool)>(
        "SELECT u.id, u.is_admin
         FROM auth_sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.expires_at > now()",
    )
    .bind(&token_hash)
    .fetch_optional(db)
    .await?;

    if row.is_some() {
        if let Err(error) = sqlx::query(
            "UPDATE auth_sessions SET last_used_at = now()
             WHERE token_hash = $1
               AND (last_used_at IS NULL OR last_used_at < now() - interval '5 minutes')",
        )
        .bind(&token_hash)
        .execute(db)
        .await
        {
            tracing::warn!(?error, "failed to refresh session activity");
        }
    }
    Ok(row.map(|(id, is_admin)| AuthenticatedSession {
        user: AuthUser { id, is_admin },
        token_hash,
    }))
}

/// Deletes the session identified by its SHA-256 hash, if any.
///
/// # Errors
///
/// Returns the underlying [`sqlx::Error`] if the delete fails.
pub async fn revoke_session_hash(db: &PgPool, token_hash: &[u8]) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM auth_sessions WHERE token_hash = $1")
        .bind(token_hash)
        .execute(db)
        .await?;
    Ok(())
}

/// Deletes the session identified by a raw cookie token, if any.
///
/// # Errors
///
/// Returns the underlying [`sqlx::Error`] if the delete fails.
pub async fn revoke_session(db: &PgPool, token: &str) -> Result<(), sqlx::Error> {
    let token_hash = hash_token(token);
    revoke_session_hash(db, &token_hash).await
}

/// Verifies credentials without revealing whether an email is registered.
///
/// # Errors
///
/// Returns a sanitized server error when the query or password verifier fails,
/// or an unauthorized error when the credentials are invalid.
pub async fn verify_credentials(
    db: &PgPool,
    email: &str,
    password: &str,
) -> Result<UserDto, ServerFnError> {
    let bounded = !email.is_empty()
        && email.len() <= EMAIL_MAX_LENGTH
        && !password.is_empty()
        && password.len() <= PASSWORD_MAX_LENGTH;
    let row = if bounded {
        sqlx::query_as::<_, (i32, String, Option<String>)>(
            "SELECT id, display_name, password_hash FROM users WHERE email = $1",
        )
        .bind(email)
        .fetch_optional(db)
        .await
        .map_err(|error| {
            tracing::error!(?error, "credential lookup failed");
            server_error("Authentication service unavailable")
        })?
    } else {
        None
    };

    let dummy_hash = dummy_password_hash()?;
    let hash = row
        .as_ref()
        .and_then(|(_, _, hash)| hash.as_deref())
        .unwrap_or(dummy_hash);
    let candidate = if bounded {
        password
    } else {
        "invalid-password"
    };
    let password_ok = verify_password(candidate, hash);
    let Some((id, display_name, Some(_))) = row.filter(|_| bounded && password_ok) else {
        return Err(unauthorized_error("Invalid email or password"));
    };
    Ok(UserDto { id, display_name })
}

/// Verifies credentials and creates a new opaque session.
///
/// # Errors
///
/// Returns the credential error or a sanitized session-store error.
pub async fn login_session(
    db: &PgPool,
    email: &str,
    password: &str,
) -> Result<(String, UserDto), ServerFnError> {
    let user = verify_credentials(db, email, password).await?;
    let token = issue_session(db, user.id).await.map_err(|error| {
        tracing::error!(?error, "session creation failed");
        server_error("Authentication service unavailable")
    })?;
    Ok((token, user))
}

/// Resolves the public identity for an authenticated user ID.
///
/// # Errors
///
/// Returns a sanitized server error when the lookup fails, or unauthorized if
/// the user no longer exists.
pub async fn user_dto(db: &PgPool, user_id: i32) -> Result<UserDto, ServerFnError> {
    let row =
        sqlx::query_as::<_, (i32, String)>("SELECT id, display_name FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(db)
            .await
            .map_err(|error| {
                tracing::error!(?error, "authenticated user lookup failed");
                server_error("Authentication service unavailable")
            })?;
    row.map(|(id, display_name)| UserDto { id, display_name })
        .ok_or_else(|| unauthorized_error("Unauthorized"))
}

/// Returns the authenticated session for the current request.
///
/// # Errors
///
/// Returns a 401 `ServerFnError` when no valid session was presented.
pub async fn current_session() -> Result<AuthenticatedSession, ServerFnError> {
    match FullstackContext::extract::<Extension<AuthenticatedSession>, _>().await {
        Ok(Extension(session)) => Ok(session),
        Err(_) => Err(unauthorized_error("Unauthorized")),
    }
}

/// Returns the authenticated user for the current request.
///
/// # Errors
///
/// Returns a 401 `ServerFnError` when no valid session was presented.
pub async fn current_user() -> Result<AuthUser, ServerFnError> {
    Ok(current_session().await?.user)
}

fn push_set_cookie(cookie: &str) -> Result<(), ServerFnError> {
    let value = HeaderValue::from_str(cookie)
        .map_err(|e| ServerFnError::new(format!("invalid cookie: {e}")))?;
    if let Some(ctx) = FullstackContext::current() {
        ctx.add_response_header(SET_COOKIE, value);
    }
    Ok(())
}

pub(crate) fn set_session_cookie(token: &str) -> Result<(), ServerFnError> {
    push_set_cookie(&format!(
        "{COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={SESSION_MAX_AGE_SECS}"
    ))
}

fn clear_session_cookie() -> Result<(), ServerFnError> {
    push_set_cookie(&format!(
        "{COOKIE_NAME}=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0"
    ))
}

pub(crate) async fn revoke_browser_session() -> Result<(), ServerFnError> {
    let jar = FullstackContext::extract::<CookieJar, _>()
        .await
        .map_err(|_| ServerFnError::new("no request context"))?;
    if let Some(cookie) = jar.get(COOKIE_NAME) {
        match pool() {
            Ok(db) => {
                if let Err(error) = revoke_session(db, cookie.value()).await {
                    tracing::warn!(?error, "failed to revoke browser session");
                }
            }
            Err(error) => tracing::warn!(?error, "session pool unavailable during logout"),
        }
    }
    clear_session_cookie()?;
    Ok(())
}

pub async fn auth_enrich_layer(jar: CookieJar, mut req: Request, next: Next) -> Response {
    let token = match req.headers().get(AUTHORIZATION) {
        Some(header) => bearer_token(Some(header)),
        None => jar.get(COOKIE_NAME).map(|cookie| cookie.value()),
    };
    if let Some(token) = token {
        match pool() {
            Ok(db) => match user_session_for_token(db, token).await {
                Ok(Some(session)) => {
                    req.extensions_mut().insert(session);
                }
                Ok(None) => {}
                Err(error) => tracing::warn!(?error, "failed to resolve request session"),
            },
            Err(error) => tracing::warn!(?error, "session pool unavailable"),
        }
    }
    next.run(req).await
}

fn bearer_token(header: Option<&HeaderValue>) -> Option<&str> {
    let token = header?.to_str().ok()?.strip_prefix("Bearer ")?;
    let valid = !token.is_empty()
        && token.len() <= SESSION_TOKEN_MAX_LENGTH
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    valid.then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[rstest::rstest]
    #[case(None, None)]
    #[case(Some("Basic abc"), None)]
    #[case(Some("Bearer"), None)]
    #[case(Some("Bearer "), None)]
    #[case(Some("Bearer  valid-token"), None)]
    #[case(Some("Bearer valid-token extra"), None)]
    #[case(Some("Bearer invalid/token"), None)]
    #[case(Some("Bearer valid-token"), Some("valid-token"))]
    fn bearer_token_cases(#[case] header: Option<&str>, #[case] expected: Option<&str>) {
        let value = header.and_then(|raw| HeaderValue::from_str(raw).ok());
        assert_eq!(bearer_token(value.as_ref()), expected);
    }

    #[test]
    fn generated_token_fits_bearer_bound() {
        let token = generate_session_token();
        let raw = ["Bearer ", token.as_str()].concat();
        let header = HeaderValue::from_str(&raw).expect("valid header");
        assert_eq!(bearer_token(Some(&header)), Some(token.as_str()));

        let oversized = [raw.as_str(), "a"].concat();
        let header = HeaderValue::from_str(&oversized).expect("valid header");
        assert_eq!(bearer_token(Some(&header)), None);
    }
}
