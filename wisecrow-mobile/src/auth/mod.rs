mod session_store;

use std::sync::Arc;

pub use session_store::{SessionStore, SessionStoreError};
use wisecrow_dto::UserDto;

#[derive(Debug, PartialEq, Eq)]
pub enum AuthState {
    Restoring,
    Anonymous,
    Authenticated(UserDto),
}

pub type SharedSessionStore = Arc<dyn SessionStore>;
