use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::traits::{BackgroundScheduler, CredentialStore, LocalStore, MobileApi};
use crate::{
    auth::AuthState,
    sync::{SyncEngine, SyncOutcome, SyncReason},
};

/// Process-wide mobile resources shared by application services and UI state.
pub struct AppServices {
    pub store: Arc<dyn LocalStore>,
    pub api: Arc<dyn MobileApi>,
    pub credentials: Arc<dyn CredentialStore>,
    pub background: Arc<dyn BackgroundScheduler>,
}

impl AppServices {
    #[must_use]
    pub fn new(
        store: impl Into<Arc<dyn LocalStore>>,
        api: impl Into<Arc<dyn MobileApi>>,
        credentials: impl Into<Arc<dyn CredentialStore>>,
        background: impl Into<Arc<dyn BackgroundScheduler>>,
    ) -> Self {
        Self {
            store: store.into(),
            api: api.into(),
            credentials: credentials.into(),
            background: background.into(),
        }
    }

    /// Runs one authenticated synchronization cycle and expires invalid sessions.
    pub async fn sync(
        &self,
        reason: SyncReason,
        cancellation: impl Into<Arc<CancellationToken>>,
        auth_state: &mut AuthState,
    ) -> SyncOutcome {
        let profile = match self.store.active_profile().await {
            Ok(Some(profile)) => profile,
            Ok(None) => return expire_auth_state(auth_state),
            Err(_) => return SyncOutcome::PermanentFailure,
        };
        match self.credentials.load(profile.id).await {
            Ok(Some(token)) => drop(token),
            Ok(None) => return expire_auth_state(auth_state),
            Err(_) => return SyncOutcome::PermanentFailure,
        }
        let engine = SyncEngine::new(
            Arc::clone(&self.store), // clone: one sync engine shares the process-wide store resource
            Arc::clone(&self.api),   // clone: one sync engine shares the authenticated API resource
            cancellation,
        );
        let outcome = engine.run(reason).await;
        if outcome == SyncOutcome::AuthenticationExpired {
            *auth_state = AuthState::Anonymous;
            if self.credentials.delete(profile.id).await.is_err() {
                return SyncOutcome::PermanentFailure;
            }
        }
        outcome
    }
}

fn expire_auth_state(auth_state: &mut AuthState) -> SyncOutcome {
    *auth_state = AuthState::Anonymous;
    SyncOutcome::AuthenticationExpired
}
