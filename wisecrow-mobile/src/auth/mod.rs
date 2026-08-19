use crate::storage::models::ProfileIdentity;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthState {
    Restoring,
    Anonymous,
    Authenticated(ProfileIdentity),
}
