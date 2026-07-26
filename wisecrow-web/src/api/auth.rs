use dioxus::prelude::*;
use wisecrow_dto::{MobileSessionDto, UserDto};

/// Authenticates a browser session and sets its secure cookie.
///
/// # Errors
///
/// Returns an authentication or session-store error.
#[post("/api/auth/login")]
pub async fn login(email: String, password: String) -> Result<UserDto, ServerFnError> {
    let db = crate::server::pool()?;
    let (token, user) = crate::server::auth::login_session(db, &email, &password).await?;
    crate::server::auth::set_session_cookie(&token)?;
    Ok(user)
}

/// Revokes the browser session and clears its secure cookie.
///
/// # Errors
///
/// Returns an error when the request context cannot update the cookie.
#[post("/api/auth/logout")]
pub async fn logout() -> Result<(), ServerFnError> {
    crate::server::auth::revoke_browser_session().await
}

/// Authenticates a native client and returns its bearer session once.
///
/// # Errors
///
/// Returns an authentication or session-store error.
#[post("/api/mobile/login")]
pub async fn mobile_login(
    email: String,
    password: String,
) -> Result<MobileSessionDto, ServerFnError> {
    let db = crate::server::pool()?;
    let (token, user) = crate::server::auth::login_session(db, &email, &password).await?;
    Ok(MobileSessionDto { token, user })
}

/// Restores the identity associated with the current bearer session.
///
/// # Errors
///
/// Returns unauthorized for an invalid session, or a sanitized lookup error.
#[post("/api/mobile/me")]
pub async fn mobile_me() -> Result<UserDto, ServerFnError> {
    let user = crate::server::auth::current_user().await?;
    crate::server::auth::user_dto(crate::server::pool()?, user.id).await
}

/// Revokes the current bearer session.
///
/// # Errors
///
/// Returns unauthorized for an invalid session, or a sanitized store error.
#[post("/api/mobile/logout")]
pub async fn mobile_logout() -> Result<(), ServerFnError> {
    let session = crate::server::auth::current_session().await?;
    crate::server::auth::revoke_session_hash(crate::server::pool()?, session.token_hash())
        .await
        .map_err(|error| {
            tracing::error!(?error, "native session revocation failed");
            ServerFnError::new("Authentication service unavailable")
        })
}
