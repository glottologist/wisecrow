//! Authentication supplied by native hosts; browsers use session cookies.

use std::{future::Future, pin::Pin, sync::Arc};

use dioxus::prelude::*;
use wisecrow_dto::UserDto;

/// An authentication operation tied to its borrowed inputs.
pub type SessionFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ServerFnError>> + 'a>>;

/// Host-owned authentication and secure credential persistence.
pub trait SessionClient: Send + Sync {
    fn login<'a>(&'a self, email: &'a str, password: &'a str) -> SessionFuture<'a, UserDto>;
    fn logout(&self) -> SessionFuture<'_, ()>;
}

/// Shared sign-in state for a host that supplies native credentials.
#[derive(Clone)]
pub struct SessionContext {
    pub client: Arc<dyn SessionClient>,
    pub authenticated: Signal<bool>,
}

/// Native document selection with a byte limit enforced before returning data.
pub trait PdfPicker: Send + Sync {
    fn pick(&self, maximum_bytes: u64) -> SessionFuture<'_, Option<Vec<u8>>>;
}

pub(crate) async fn login(email: String, password: String) -> Result<UserDto, ServerFnError> {
    if let Some(mut session) = try_consume_context::<SessionContext>() {
        let user = session.client.login(&email, &password).await?;
        session.authenticated.set(true);
        Ok(user)
    } else {
        crate::api::auth::login(email, password).await
    }
}

pub(crate) async fn logout() -> Result<(), ServerFnError> {
    if let Some(mut session) = try_consume_context::<SessionContext>() {
        session.client.logout().await?;
        session.authenticated.set(false);
        Ok(())
    } else {
        crate::api::auth::logout().await
    }
}

#[cfg(all(test, feature = "server"))]
mod tests {
    use super::*;

    struct SignedOut;

    impl SessionClient for SignedOut {
        fn login<'a>(&'a self, _: &'a str, _: &'a str) -> SessionFuture<'a, UserDto> {
            Box::pin(async { Err(ServerFnError::new("No test credentials")) })
        }

        fn logout(&self) -> SessionFuture<'_, ()> {
            Box::pin(async { Ok(()) })
        }
    }

    fn signed_out_app() -> Element {
        let authenticated = use_signal(|| false);
        use_context_provider(|| SessionContext {
            client: Arc::new(SignedOut),
            authenticated,
        });
        rsx! { crate::app {} }
    }

    #[test]
    fn a_signed_out_native_host_cannot_render_the_protected_home() {
        let mut dom = VirtualDom::new(signed_out_app);
        dom.rebuild_in_place();
        let html = dioxus::ssr::render(&dom);
        assert!(html.contains("Sign in"));
        assert!(html.contains("type=\"password\""));
        assert!(!html.contains("Start a session"));
    }
}
