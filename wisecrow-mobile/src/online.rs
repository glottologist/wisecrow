//! Native authentication for the shared web interface.

use std::{sync::Arc, time::Duration};

use dioxus::prelude::*;
use dioxus_fullstack as fullstack;
use uuid::Uuid;
use wisecrow_dto::{MobileSessionDto, UserDto};
use wisecrow_web::session::{SessionClient, SessionContext, SessionFuture};

use crate::application::{CredentialStore, FilePicker, MobileError};

const PROFILE_ID: Uuid = Uuid::from_u128(0x4630759a_7596_4396_8c54_b08365f60dae);
const SERVER_URL: &str = "https://wisecrow.glottologist.co.uk:8443";

/// Configures the single trusted deployment before any server request is made.
pub(crate) fn configure() -> Result<(), ServerFnError> {
    let client = fullstack::reqwest::Client::builder()
        .https_only(true)
        .redirect(fullstack::reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|_| ServerFnError::new("Could not initialize secure networking"))?;
    fullstack::GLOBAL_REQUEST_CLIENT
        .set(client)
        .map_err(|_| ServerFnError::new("Networking was already initialized"))?;
    fullstack::set_server_url(SERVER_URL);
    Ok(())
}

trait SessionApi: Send + Sync {
    fn login<'a>(
        &'a self,
        email: &'a str,
        password: &'a str,
    ) -> SessionFuture<'a, MobileSessionDto>;
    fn me(&self) -> SessionFuture<'_, UserDto>;
    fn logout(&self) -> SessionFuture<'_, ()>;
}

struct ServerSession;

impl SessionApi for ServerSession {
    fn login<'a>(
        &'a self,
        email: &'a str,
        password: &'a str,
    ) -> SessionFuture<'a, MobileSessionDto> {
        Box::pin(wisecrow_web::api::auth::mobile_login(
            email.into(),
            password.into(),
        ))
    }

    fn me(&self) -> SessionFuture<'_, UserDto> {
        Box::pin(wisecrow_web::api::auth::mobile_me())
    }

    fn logout(&self) -> SessionFuture<'_, ()> {
        Box::pin(wisecrow_web::api::auth::mobile_logout())
    }
}

struct OnlineSession {
    credentials: Arc<dyn CredentialStore>,
    api: Arc<dyn SessionApi>,
    operation: tokio::sync::Mutex<()>,
}

impl OnlineSession {
    fn new(credentials: Arc<dyn CredentialStore>, api: Arc<dyn SessionApi>) -> Self {
        Self {
            credentials,
            api,
            operation: tokio::sync::Mutex::new(()),
        }
    }

    async fn restore(&self) -> Result<Option<UserDto>, ServerFnError> {
        let _operation = self.operation.lock().await;
        fullstack::clear_request_headers();
        let Some(token) = self
            .credentials
            .load(PROFILE_ID)
            .await
            .map_err(credential_error)?
        else {
            return Ok(None);
        };
        let headers = match authorization_headers(&token) {
            Ok(headers) => headers,
            Err(_) => {
                self.credentials
                    .delete(PROFILE_ID)
                    .await
                    .map_err(credential_error)?;
                return Ok(None);
            }
        };
        fullstack::set_request_headers(headers);
        match self.api.me().await {
            Ok(user) => Ok(Some(user)),
            Err(error) => {
                fullstack::clear_request_headers();
                if authentication_expired(&error) {
                    self.credentials
                        .delete(PROFILE_ID)
                        .await
                        .map_err(credential_error)?;
                    Ok(None)
                } else {
                    Err(error)
                }
            }
        }
    }
}

impl SessionClient for OnlineSession {
    fn login<'a>(&'a self, email: &'a str, password: &'a str) -> SessionFuture<'a, UserDto> {
        Box::pin(async move {
            let _operation = self.operation.lock().await;
            if email.len() > 254
                || !email.contains('@')
                || email.chars().any(char::is_control)
                || password.is_empty()
                || password.len() > 1024
            {
                return Err(ServerFnError::new("Enter a valid email and password"));
            }
            let session = self.api.login(email, password).await?;
            let headers = authorization_headers(&session.token)?;
            if let Err(error) = self.credentials.save(PROFILE_ID, &session.token).await {
                fullstack::set_request_headers(headers);
                if self.api.logout().await.is_err() {
                    tracing::warn!("Could not revoke session after credential storage failed");
                }
                fullstack::clear_request_headers();
                return Err(credential_error(error));
            }
            fullstack::set_request_headers(headers);
            Ok(session.user)
        })
    }

    fn logout(&self) -> SessionFuture<'_, ()> {
        Box::pin(async {
            let _operation = self.operation.lock().await;
            if self.api.logout().await.is_err() {
                tracing::warn!("Remote logout unavailable; removing local session");
            }
            fullstack::clear_request_headers();
            self.credentials
                .delete(PROFILE_ID)
                .await
                .map_err(credential_error)
        })
    }
}

fn credential_error(_: MobileError) -> ServerFnError {
    ServerFnError::new("Secure credential storage is unavailable")
}

fn authentication_expired(error: &ServerFnError) -> bool {
    matches!(
        error,
        ServerFnError::ServerError {
            code: 401 | 403,
            ..
        } | ServerFnError::Request(fullstack::RequestError::Status(_, 401 | 403))
    )
}

fn authorization_headers(token: &str) -> Result<fullstack::HeaderMap, ServerFnError> {
    if token.is_empty()
        || token.len() > 512
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ServerFnError::new("Invalid session credential"));
    }
    let mut value = fullstack::HeaderValue::from_str(&["Bearer ", token].concat())
        .map_err(|_| ServerFnError::new("Invalid session credential"))?;
    value.set_sensitive(true);
    let mut headers = fullstack::HeaderMap::new();
    headers.insert(fullstack::http::header::AUTHORIZATION, value);
    Ok(headers)
}

struct NativePdfPicker(Arc<dyn FilePicker>);

impl wisecrow_web::session::PdfPicker for NativePdfPicker {
    fn pick(&self, maximum_bytes: u64) -> SessionFuture<'_, Option<Vec<u8>>> {
        Box::pin(async move {
            self.0
                .pick_pdf(maximum_bytes)
                .await
                .map(|file| file.map(|file| file.bytes))
                .map_err(|_| {
                    ServerFnError::new("Could not open this PDF. Choose a PDF smaller than 80 MB.")
                })
        })
    }
}

pub(crate) fn app() -> Element {
    let credentials = use_context::<Arc<dyn CredentialStore>>();
    let picker = use_context::<Arc<dyn FilePicker>>();
    use_context_provider(|| {
        Arc::new(NativePdfPicker(picker)) as Arc<dyn wisecrow_web::session::PdfPicker>
    });
    let session = use_hook(|| Arc::new(OnlineSession::new(credentials, Arc::new(ServerSession))));
    let mut authenticated = use_signal(|| false);
    use_context_provider(|| SessionContext {
        client: session.clone(), // clone: UI and startup share one serialized session coordinator
        authenticated,
    });
    let mut restored = use_resource(move || {
        let session = Arc::clone(&session); // clone: the restore future owns its shared coordinator
        async move {
            let result = session.restore().await?;
            authenticated.set(result.is_some());
            Ok::<_, ServerFnError>(())
        }
    });
    match &*restored.read_unchecked() {
        Some(Ok(())) => rsx! { wisecrow_web::app {} },
        Some(Err(_)) => rsx! {
            div { style: "padding:2rem;font-family:system-ui",
                h1 { "Wise Crow" }
                p { "Could not restore your session. Check your connection and try again." }
                button { onclick: move |_| restored.restart(), "Retry" }
            }
        },
        None => rsx! { p { style: "padding:2rem;font-family:system-ui", "Opening Wise Crow…" } },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Credentials {
        token: Mutex<Option<String>>,
        fail_save: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl CredentialStore for Credentials {
        async fn load(&self, _: Uuid) -> Result<Option<String>, MobileError> {
            Ok(self.token.lock().expect("test lock").clone())
        }
        async fn save(&self, _: Uuid, token: &str) -> Result<(), MobileError> {
            if self.fail_save.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(MobileError::Credentials);
            }
            *self.token.lock().expect("test lock") = Some(token.into());
            Ok(())
        }
        async fn delete(&self, _: Uuid) -> Result<(), MobileError> {
            *self.token.lock().expect("test lock") = None;
            Ok(())
        }
    }

    struct Api {
        status: std::sync::atomic::AtomicU16,
    }

    fn user() -> UserDto {
        UserDto {
            id: 7,
            display_name: String::from("Learner"),
        }
    }

    impl SessionApi for Api {
        fn login<'a>(&'a self, _: &'a str, _: &'a str) -> SessionFuture<'a, MobileSessionDto> {
            Box::pin(async {
                Ok(MobileSessionDto {
                    token: String::from("valid-test-token"),
                    user: user(),
                })
            })
        }
        fn me(&self) -> SessionFuture<'_, UserDto> {
            Box::pin(async {
                let status = self.status.load(std::sync::atomic::Ordering::Relaxed);
                if status != 200 {
                    Err(ServerFnError::ServerError {
                        message: String::from("expired"),
                        code: status,
                        details: None,
                    })
                } else {
                    Ok(user())
                }
            })
        }
        fn logout(&self) -> SessionFuture<'_, ()> {
            Box::pin(async { Err(ServerFnError::new("offline")) })
        }
    }

    #[tokio::test]
    async fn native_sessions_persist_restore_expire_and_clear_after_offline_logout() {
        let credentials = Arc::new(Credentials::default());
        let api = Arc::new(Api { status: 200.into() });
        let session = OnlineSession::new(credentials.clone(), api.clone());
        assert_eq!(session.restore().await.expect("anonymous restore"), None);
        assert_eq!(
            session
                .login("learner@example.test", "password")
                .await
                .expect("login"),
            user()
        );
        assert!(fullstack::get_request_headers().contains_key("authorization"));
        assert_eq!(session.restore().await.expect("restore"), Some(user()));
        session
            .logout()
            .await
            .expect("offline logout clears local credentials");
        assert!(credentials.load(PROFILE_ID).await.expect("load").is_none());
        assert!(!fullstack::get_request_headers().contains_key("authorization"));

        credentials
            .fail_save
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(session
            .login("learner@example.test", "password")
            .await
            .is_err());
        assert!(!fullstack::get_request_headers().contains_key("authorization"));
        credentials
            .fail_save
            .store(false, std::sync::atomic::Ordering::Relaxed);
        session
            .login("learner@example.test", "password")
            .await
            .expect("login again");
        api.status.store(503, std::sync::atomic::Ordering::Relaxed);
        assert!(session.restore().await.is_err());
        assert!(credentials
            .load(PROFILE_ID)
            .await
            .expect("saved offline credential")
            .is_some());
        assert!(!fullstack::get_request_headers().contains_key("authorization"));
        api.status.store(401, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(session.restore().await.expect("expired session"), None);
        assert!(credentials.load(PROFILE_ID).await.expect("load").is_none());
        assert!(!fullstack::get_request_headers().contains_key("authorization"));
    }

    proptest::proptest! {
        #[test]
        fn bearer_header_accepts_only_bounded_url_safe_tokens(token in ".{0,600}") {
            let valid = !token.is_empty() && token.len() <= 512
                && token.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
            proptest::prop_assert_eq!(authorization_headers(&token).is_ok(), valid);
        }
    }
}
