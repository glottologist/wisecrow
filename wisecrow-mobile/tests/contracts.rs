use wisecrow_dto::{MobileSessionDto, UserDto};
use wisecrow_mobile::auth::{AuthState, SessionStore, SessionStoreError};
use wisecrow_mobile::transport::MobileConfig;

struct ProbeStore;

impl SessionStore for ProbeStore {
    fn load(&self) -> Result<Option<String>, SessionStoreError> {
        Ok(None)
    }

    fn save(&self, _token: &str) -> Result<(), SessionStoreError> {
        Ok(())
    }

    fn delete(&self) -> Result<(), SessionStoreError> {
        Ok(())
    }
}

#[test]
fn public_contracts_are_available() {
    let state = AuthState::Authenticated(UserDto {
        id: 7,
        display_name: "Test".to_owned(),
    });
    assert!(matches!(state, AuthState::Authenticated(_)));

    let session = MobileSessionDto {
        token: "secret".to_owned(),
        user: UserDto {
            id: 7,
            display_name: "Test".to_owned(),
        },
    };
    assert_eq!(session.user.id, 7);

    let _store: &dyn SessionStore = &ProbeStore;
    let _config_type = std::any::TypeId::of::<MobileConfig>();
}
