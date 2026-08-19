use std::sync::Arc;

use chrono::Utc;
use wisecrow_dto::{
    DeviceRegistrationRequestDto, MobileCapabilitiesDto, MobileSessionDto, UserDto,
    MOBILE_PROTOCOL_VERSION,
};

use super::{ApiFactory, CredentialStore, LocalStore, MobileApi, MobileError};
use crate::{
    auth::AuthState,
    storage::models::{Profile, ProfileIdentity},
    transport::ServerOrigin,
};

/// Coordinates validated profiles, secure credentials, and authenticated identity.
pub struct ProfileService {
    store: Arc<dyn LocalStore>,
    api_factory: Arc<dyn ApiFactory>,
    credentials: Arc<dyn CredentialStore>,
}

impl ProfileService {
    #[must_use]
    pub fn new(
        store: impl Into<Arc<dyn LocalStore>>,
        api_factory: impl Into<Arc<dyn ApiFactory>>,
        credentials: impl Into<Arc<dyn CredentialStore>>,
    ) -> Self {
        Self {
            store: store.into(),
            api_factory: api_factory.into(),
            credentials: credentials.into(),
        }
    }

    /// Verifies that a profile has a safe origin and compatible server protocol.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, transport, or compatibility error.
    pub async fn verify_server(
        &self,
        profile: &Profile,
    ) -> Result<MobileCapabilitiesDto, MobileError> {
        let api = self.validated_api(profile)?;
        compatible_capabilities(api.as_ref()).await
    }

    /// Authenticates, registers this installation, and persists the identity atomically.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, authentication, device, credential, or storage error.
    pub async fn login(
        &self,
        profile: &Profile,
        email: &str,
        password: &str,
        device_display_name: &str,
    ) -> Result<AuthState, MobileError> {
        validate_login_input(email, password, device_display_name)?;
        let api = self.validated_api(profile)?;
        compatible_capabilities(api.as_ref()).await?;
        let MobileSessionDto { token, user } = api.login(email, password).await?;
        self.credentials.save(profile.id, &token).await?;
        let result = self
            .finish_login(profile, api.as_ref(), user, device_display_name)
            .await;
        if result.is_err() {
            self.delete_credentials_after_failure(profile.id).await;
        }
        result.map(AuthState::Authenticated)
    }

    /// Restores authenticated state for the active profile when its token is valid.
    ///
    /// # Errors
    ///
    /// Returns a typed credential, transport, or storage error.
    pub async fn restore(&self) -> Result<AuthState, MobileError> {
        let Some(profile) = self.store.active_profile().await? else {
            return Ok(AuthState::Anonymous);
        };
        if self.credentials.load(profile.id).await?.is_none() {
            return Ok(AuthState::Anonymous);
        }
        let api = self.validated_api(&profile)?;
        let user = match api.me().await {
            Ok(user) => user,
            Err(MobileError::Authentication | MobileError::DeviceRevoked) => {
                self.credentials.delete(profile.id).await?;
                return Ok(AuthState::Anonymous);
            }
            Err(error) => return Err(error),
        };
        self.restored_identity(profile.id, user.id).await
    }

    /// Revokes the server session before deleting the local credential.
    ///
    /// # Errors
    ///
    /// Returns a storage or credential error. A remote logout failure is non-fatal.
    pub async fn logout(&self) -> Result<AuthState, MobileError> {
        let Some(profile) = self.store.active_profile().await? else {
            return Ok(AuthState::Anonymous);
        };
        self.revoke_remote_session(&profile).await;
        self.credentials.delete(profile.id).await?;
        Ok(AuthState::Anonymous)
    }

    /// Activates another profile and restores its authenticated identity.
    ///
    /// # Errors
    ///
    /// Returns a typed storage, credential, or transport error.
    pub async fn switch_profile(&self, profile_id: uuid::Uuid) -> Result<AuthState, MobileError> {
        self.store.activate_profile(profile_id).await?;
        self.restore().await
    }

    async fn finish_login(
        &self,
        profile: &Profile,
        api: &dyn MobileApi,
        login_user: UserDto,
        device_display_name: &str,
    ) -> Result<ProfileIdentity, MobileError> {
        let user = api.me().await?;
        if user != login_user {
            return Err(MobileError::Conflict(String::from(
                "login identity does not match authenticated identity",
            )));
        }
        let device_id = self.installation_id(profile.id, user.id).await?;
        let request = device_request(device_id, device_display_name);
        let registered = api.register_device(&request).await?;
        if registered.revoked_at.is_some() {
            return Err(MobileError::DeviceRevoked);
        }
        let identity = active_identity(profile, user, registered.device_id);
        self.store.save_profile_identity(&identity).await?;
        Ok(identity)
    }

    async fn installation_id(
        &self,
        profile_id: uuid::Uuid,
        user_id: i32,
    ) -> Result<uuid::Uuid, MobileError> {
        Ok(self
            .store
            .profile_identity(profile_id, user_id)
            .await?
            .map_or_else(uuid::Uuid::new_v4, |identity| identity.device_id))
    }

    async fn restored_identity(
        &self,
        profile_id: uuid::Uuid,
        user_id: i32,
    ) -> Result<AuthState, MobileError> {
        match self.store.profile_identity(profile_id, user_id).await? {
            Some(identity) => Ok(AuthState::Authenticated(identity)),
            None => {
                self.credentials.delete(profile_id).await?;
                Ok(AuthState::Anonymous)
            }
        }
    }

    fn validated_api(&self, profile: &Profile) -> Result<Arc<dyn MobileApi>, MobileError> {
        let origin = ServerOrigin::parse(&profile.origin)?;
        if origin.as_url().as_str() != profile.origin {
            return Err(MobileError::InvalidInput(String::from(
                "server origin must be normalized",
            )));
        }
        self.api_factory.create(profile)
    }

    async fn delete_credentials_after_failure(&self, profile_id: uuid::Uuid) {
        if let Err(error) = self.credentials.delete(profile_id).await {
            tracing::warn!(error = %error, "failed to remove credentials after login failure");
        }
    }

    async fn revoke_remote_session(&self, profile: &Profile) {
        match self.validated_api(profile) {
            Ok(api) => {
                if let Err(error) = api.logout().await {
                    tracing::warn!(error = %error, "remote logout failed; removing local credentials");
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "remote logout unavailable; removing local credentials");
            }
        }
    }
}

async fn compatible_capabilities(
    api: &dyn MobileApi,
) -> Result<MobileCapabilitiesDto, MobileError> {
    let capabilities = api.capabilities().await?;
    if capabilities.protocol_version != MOBILE_PROTOCOL_VERSION {
        return Err(MobileError::UnsupportedProtocol {
            required: MOBILE_PROTOCOL_VERSION,
            actual: capabilities.protocol_version,
        });
    }
    Ok(capabilities)
}

fn active_identity(profile: &Profile, user: UserDto, device_id: uuid::Uuid) -> ProfileIdentity {
    let mut active_profile = profile.clone(); // clone: authenticated state must own the caller-provided profile
    active_profile.active = true;
    active_profile.updated_at = Utc::now();
    ProfileIdentity {
        profile: active_profile,
        user,
        device_id,
    }
}

fn device_request(
    device_id: uuid::Uuid,
    device_display_name: &str,
) -> DeviceRegistrationRequestDto {
    DeviceRegistrationRequestDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        device_id,
        display_name: String::from(device_display_name),
    }
}

fn validate_login_input(
    email: &str,
    password: &str,
    device_display_name: &str,
) -> Result<(), MobileError> {
    let valid_email = !email.is_empty() && email.len() <= 254 && email.contains('@');
    let valid_password = !password.is_empty() && password.len() <= 1024;
    let valid_device_name = !device_display_name.is_empty()
        && device_display_name.len() <= 128
        && !device_display_name.chars().any(char::is_control);
    if !valid_email || !valid_password || !valid_device_name {
        return Err(MobileError::InvalidInput(String::from(
            "login fields are invalid",
        )));
    }
    Ok(())
}
