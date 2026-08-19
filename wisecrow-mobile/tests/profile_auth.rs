use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use proptest::prelude::*;
use tempfile::tempdir;
use uuid::Uuid;
use wisecrow_dto::{
    CachedQuizDto, CardChangePageDto, CardChangeRequestDto, CardSnapshotDto, CorpusChangePageDto,
    CorpusChangeRequestDto, CorpusPageDto, CorpusSnapshotRequestDto, DeviceRegistrationRequestDto,
    LanguageInfo, LanguagePairDto, MobileCapabilitiesDto, MobileSessionDto, NbackBatchRequestDto,
    NbackBatchResponseDto, NbackSessionUploadDto, RegisteredDeviceDto, ReviewBatchRequestDto,
    ReviewBatchResponseDto, ReviewEventDto, UserDto, MOBILE_PROTOCOL_VERSION,
};
use wisecrow_mobile::application::{
    ApiFactory, ContentRepository, CorpusRepository, CredentialStore, LearningRepository,
    LocalStore, MobileApi, MobileError, ProfileRepository, ProfileService,
};
use wisecrow_mobile::auth::AuthState;
use wisecrow_mobile::storage::models::{
    LocalAnswer, LocalSession, LocalSessionRequest, MediaEntry, MediaRegistration, MediaType,
    PairStatus, PairSyncState, Profile, ProfileIdentity, SyncErrorKind, SyncPhase,
};
use wisecrow_mobile::storage::SqliteStore;
use wisecrow_mobile::transport::ServerOrigin;

proptest! {
    #[test]
    fn server_origin_rejects_unsafe_inputs(label in "[a-z]{1,12}", case in 0u8..5) {
        let input = rejected_origin(&label, case);
        prop_assert!(ServerOrigin::parse(&input).is_err());
    }
}

fn rejected_origin(label: &str, case: u8) -> String {
    let mut origin = match case {
        0 => String::from("http://"),
        1 => String::from("https://user@"),
        2..=4 => String::from("https://"),
        _ => String::new(),
    };
    origin.push_str(label);
    origin.push_str(".test");
    match case {
        2 => origin.push_str("/?query=value"),
        3 => origin.push_str("/#fragment"),
        4 => origin.insert_str(0, "://"),
        _ => {}
    }
    origin
}

#[test]
fn server_origin_normalizes_prefix_and_validates_endpoint_segments() {
    let origin = ServerOrigin::parse("https://example.test/wisecrow").expect("valid origin");
    assert_eq!(origin.as_url().as_str(), "https://example.test/wisecrow/");
    let repeated =
        ServerOrigin::parse("https://example.test/wisecrow///").expect("repeated slashes");
    assert_eq!(repeated.as_url().as_str(), "https://example.test/wisecrow/");
    assert_eq!(
        origin
            .endpoint(&["api", "mobile", "capabilities"])
            .expect("valid endpoint")
            .as_str(),
        "https://example.test/wisecrow/api/mobile/capabilities"
    );
    assert!(origin.endpoint(&["api/mobile"]).is_err());
    assert!(origin.endpoint(&[".."]).is_err());
    assert!(origin.endpoint(&["%2F"]).is_err());
}

#[derive(Clone, Copy)]
struct ApiBehavior {
    protocol_version: u16,
    revoked: bool,
    logout_fails: bool,
}

impl Default for ApiBehavior {
    fn default() -> Self {
        Self {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            revoked: false,
            logout_fails: false,
        }
    }
}

struct FakeApi {
    behavior: ApiBehavior,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl FakeApi {
    fn record(&self, call: &'static str) {
        self.calls.lock().expect("call log").push(call);
    }
}

#[async_trait]
impl MobileApi for FakeApi {
    async fn capabilities(&self) -> Result<MobileCapabilitiesDto, MobileError> {
        self.record("capabilities");
        Ok(MobileCapabilitiesDto {
            protocol_version: self.behavior.protocol_version,
            supported_features: Vec::new(),
            max_snapshot_page: 100,
            max_review_batch: 100,
            max_nback_batch: 10,
            server_version: String::from("test"),
        })
    }

    async fn login(&self, _email: &str, _password: &str) -> Result<MobileSessionDto, MobileError> {
        self.record("login");
        Ok(MobileSessionDto {
            token: String::from("token"),
            user: test_user(),
        })
    }

    async fn me(&self) -> Result<UserDto, MobileError> {
        self.record("me");
        Ok(test_user())
    }

    async fn languages(&self) -> Result<Vec<LanguageInfo>, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn logout(&self) -> Result<(), MobileError> {
        self.record("logout");
        if self.behavior.logout_fails {
            return Err(MobileError::Retryable);
        }
        Ok(())
    }

    async fn register_device(
        &self,
        request: &DeviceRegistrationRequestDto,
    ) -> Result<RegisteredDeviceDto, MobileError> {
        self.record("register_device");
        let now = Utc::now();
        Ok(RegisteredDeviceDto {
            device_id: request.device_id,
            display_name: request.display_name.clone(),
            created_at: now,
            last_seen_at: now,
            revoked_at: self.behavior.revoked.then_some(now),
        })
    }

    async fn corpus_snapshot(
        &self,
        _request: &CorpusSnapshotRequestDto,
    ) -> Result<CorpusPageDto, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn corpus_changes(
        &self,
        _request: &CorpusChangeRequestDto,
    ) -> Result<CorpusChangePageDto, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn card_changes(
        &self,
        _request: &CardChangeRequestDto,
    ) -> Result<CardChangePageDto, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn upload_reviews(
        &self,
        _request: &ReviewBatchRequestDto,
    ) -> Result<ReviewBatchResponseDto, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn upload_nback(
        &self,
        _request: &NbackBatchRequestDto,
    ) -> Result<NbackBatchResponseDto, MobileError> {
        Err(MobileError::Unsupported)
    }
}

struct FakeApiFactory {
    api: Arc<dyn MobileApi>,
}

impl ApiFactory for FakeApiFactory {
    fn create(&self, _profile: &Profile) -> Result<Arc<dyn MobileApi>, MobileError> {
        Ok(Arc::clone(&self.api))
    }
}

struct FakeCredentials {
    token: Mutex<Option<String>>,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl FakeCredentials {
    fn has_token(&self) -> bool {
        self.token.lock().expect("token").is_some()
    }
}

#[async_trait]
impl CredentialStore for FakeCredentials {
    async fn load(&self, _profile_id: Uuid) -> Result<Option<String>, MobileError> {
        self.calls
            .lock()
            .expect("call log")
            .push("credentials_load");
        Ok(self.token.lock().expect("token").clone())
    }

    async fn save(&self, _profile_id: Uuid, token: &str) -> Result<(), MobileError> {
        self.calls
            .lock()
            .expect("call log")
            .push("credentials_save");
        *self.token.lock().expect("token") = Some(String::from(token));
        Ok(())
    }

    async fn delete(&self, _profile_id: Uuid) -> Result<(), MobileError> {
        self.calls
            .lock()
            .expect("call log")
            .push("credentials_delete");
        *self.token.lock().expect("token") = None;
        Ok(())
    }
}

struct FakeStore {
    identity: Mutex<Option<ProfileIdentity>>,
    fail_identity_save: bool,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

#[async_trait]
impl ProfileRepository for FakeStore {
    async fn active_profile(&self) -> Result<Option<Profile>, MobileError> {
        self.calls.lock().expect("call log").push("active_profile");
        Ok(self
            .identity
            .lock()
            .expect("identity")
            .as_ref()
            .map(|identity| identity.profile.clone()))
    }

    async fn active_identity(&self) -> Result<Option<ProfileIdentity>, MobileError> {
        Ok(self.identity.lock().expect("identity").clone())
    }

    async fn save_profile(&self, _profile: &Profile) -> Result<(), MobileError> {
        self.calls.lock().expect("call log").push("profile_save");
        Ok(())
    }

    async fn activate_profile(&self, _profile_id: Uuid) -> Result<(), MobileError> {
        self.calls
            .lock()
            .expect("call log")
            .push("profile_activate");
        Ok(())
    }

    async fn profile_identity(
        &self,
        _profile_id: Uuid,
        _user_id: i32,
    ) -> Result<Option<ProfileIdentity>, MobileError> {
        self.calls.lock().expect("call log").push("identity_load");
        Ok(self.identity.lock().expect("identity").clone())
    }

    async fn save_profile_identity(&self, identity: &ProfileIdentity) -> Result<(), MobileError> {
        self.calls.lock().expect("call log").push("identity_save");
        if self.fail_identity_save {
            return Err(MobileError::Storage(sqlx::Error::RowNotFound));
        }
        *self.identity.lock().expect("identity") = Some(identity.clone());
        Ok(())
    }
}

#[async_trait]
impl CorpusRepository for FakeStore {
    async fn save_languages(&self, _languages: &[LanguageInfo]) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn begin_snapshot(
        &self,
        _pair: &LanguagePairDto,
        _snapshot_watermark: i64,
        _estimated_bytes: Option<u64>,
    ) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn apply_snapshot_page(&self, _page: &CorpusPageDto) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn apply_change_page(&self, _page: &CorpusChangePageDto) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn apply_card_page(&self, _page: &CardChangePageDto) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn pair_status(&self, _pair: &LanguagePairDto) -> Result<PairStatus, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn corpus_estimate(
        &self,
        _pair: &LanguagePairDto,
    ) -> Result<wisecrow_mobile::storage::models::CorpusEstimate, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn ranked_translations(
        &self,
        _pair: &LanguagePairDto,
        _limit: u16,
    ) -> Result<Vec<wisecrow_dto::CorpusTranslationDto>, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn translation(
        &self,
        _pair: &LanguagePairDto,
        _translation_id: i32,
    ) -> Result<Option<wisecrow_dto::CorpusTranslationDto>, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn sync_pairs(&self) -> Result<Vec<PairSyncState>, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn card_cursor(&self) -> Result<i64, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn sync_phase(&self) -> Result<SyncPhase, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn advance_sync_phase(
        &self,
        _expected: SyncPhase,
        _next: SyncPhase,
    ) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn record_sync_success(&self, _completed_at: DateTime<Utc>) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn record_sync_error(
        &self,
        _kind: SyncErrorKind,
        _failed_at: DateTime<Utc>,
    ) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }
}

#[async_trait]
impl LearningRepository for FakeStore {
    async fn create_session(
        &self,
        _request: &LocalSessionRequest,
    ) -> Result<LocalSession, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn answer(&self, _answer: &LocalAnswer) -> Result<CardSnapshotDto, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn pending_reviews(&self, _limit: u16) -> Result<Vec<ReviewEventDto>, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn apply_review_response(
        &self,
        _response: &ReviewBatchResponseDto,
    ) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }
}

#[async_trait]
impl ContentRepository for FakeStore {
    async fn save_nback(&self, _session: &NbackSessionUploadDto) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn pending_nback(&self, _limit: u16) -> Result<Vec<NbackSessionUploadDto>, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn apply_nback_response(
        &self,
        _response: &NbackBatchResponseDto,
    ) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn save_quiz(&self, _quiz: &CachedQuizDto) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn cached_quizzes(&self) -> Result<Vec<CachedQuizDto>, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn register_media(
        &self,
        _media_root: &Path,
        _registration: &MediaRegistration,
    ) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn media(
        &self,
        _media_root: &Path,
        _translation_id: i32,
        _media_type: MediaType,
        _accessed_at: DateTime<Utc>,
    ) -> Result<Option<MediaEntry>, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn media_lru(&self, _media_root: &Path) -> Result<Vec<MediaEntry>, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn eviction_candidates(
        &self,
        _media_root: &Path,
        _bytes_to_free: u64,
    ) -> Result<Vec<MediaEntry>, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn confirm_media_deleted(
        &self,
        _translation_id: i32,
        _media_type: MediaType,
    ) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }
}

struct Harness {
    service: ProfileService,
    calls: Arc<Mutex<Vec<&'static str>>>,
    store: Arc<FakeStore>,
    credentials: Arc<FakeCredentials>,
}

impl Harness {
    fn new(
        behavior: ApiBehavior,
        identity: Option<ProfileIdentity>,
        token: Option<&str>,
        fail_identity_save: bool,
    ) -> Self {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let api: Arc<dyn MobileApi> = Arc::new(FakeApi {
            behavior,
            calls: Arc::clone(&calls),
        });
        let factory: Arc<dyn ApiFactory> = Arc::new(FakeApiFactory { api });
        let store = Arc::new(FakeStore {
            identity: Mutex::new(identity),
            fail_identity_save,
            calls: Arc::clone(&calls),
        });
        let credentials = Arc::new(FakeCredentials {
            token: Mutex::new(token.map(String::from)),
            calls: Arc::clone(&calls),
        });
        let store_resource: Arc<dyn LocalStore> = store.clone();
        let credential_resource: Arc<dyn CredentialStore> = credentials.clone();
        let service = ProfileService::new(store_resource, factory, credential_resource);
        Self {
            service,
            calls,
            store,
            credentials,
        }
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().expect("call log").clone()
    }
}

#[tokio::test]
async fn login_checks_capabilities_and_reuses_the_installation_id() {
    let profile = test_profile();
    let installation_id = Uuid::new_v4();
    let identity = ProfileIdentity {
        profile: profile.clone(),
        user: test_user(),
        device_id: installation_id,
    };
    let harness = Harness::new(ApiBehavior::default(), Some(identity), None, false);

    let state = harness
        .service
        .login(&profile, "person@example.test", "secret", "Test device")
        .await
        .expect("login");

    assert!(matches!(
        state,
        AuthState::Authenticated(ref identity) if identity.device_id == installation_id
    ));
    assert_eq!(
        harness.calls(),
        [
            "capabilities",
            "login",
            "credentials_save",
            "me",
            "identity_load",
            "register_device",
            "identity_save",
        ]
    );
}

#[tokio::test]
async fn unsupported_protocol_prevents_login() {
    let behavior = ApiBehavior {
        protocol_version: MOBILE_PROTOCOL_VERSION.saturating_add(1),
        ..ApiBehavior::default()
    };
    let harness = Harness::new(behavior, None, None, false);

    let result = harness
        .service
        .login(
            &test_profile(),
            "person@example.test",
            "secret",
            "Test device",
        )
        .await;

    assert!(matches!(
        result,
        Err(MobileError::UnsupportedProtocol { .. })
    ));
    assert_eq!(harness.calls(), ["capabilities"]);
}

#[tokio::test]
async fn revoked_device_never_exposes_authenticated_state() {
    let behavior = ApiBehavior {
        revoked: true,
        ..ApiBehavior::default()
    };
    let harness = Harness::new(behavior, None, None, false);

    let result = harness
        .service
        .login(
            &test_profile(),
            "person@example.test",
            "secret",
            "Test device",
        )
        .await;

    assert!(matches!(result, Err(MobileError::DeviceRevoked)));
    assert!(!harness.credentials.has_token());
    assert!(harness.store.identity.lock().expect("identity").is_none());
}

#[tokio::test]
async fn failed_identity_transaction_removes_saved_credentials() {
    let harness = Harness::new(ApiBehavior::default(), None, None, true);

    let result = harness
        .service
        .login(
            &test_profile(),
            "person@example.test",
            "secret",
            "Test device",
        )
        .await;

    assert!(matches!(result, Err(MobileError::Storage(_))));
    assert!(!harness.credentials.has_token());
    assert_eq!(harness.calls().last(), Some(&"credentials_delete"));
}

#[tokio::test]
async fn logout_revokes_server_first_and_deletes_token_when_offline() {
    let identity = ProfileIdentity {
        profile: test_profile(),
        user: test_user(),
        device_id: Uuid::new_v4(),
    };
    let behavior = ApiBehavior {
        logout_fails: true,
        ..ApiBehavior::default()
    };
    let harness = Harness::new(behavior, Some(identity), Some("token"), false);

    let state = harness.service.logout().await.expect("local logout");

    assert_eq!(state, AuthState::Anonymous);
    assert!(!harness.credentials.has_token());
    assert_eq!(
        harness.calls(),
        ["active_profile", "logout", "credentials_delete"]
    );
}

#[tokio::test]
async fn restore_and_switch_use_the_active_profile_identity() {
    let identity = ProfileIdentity {
        profile: test_profile(),
        user: test_user(),
        device_id: Uuid::new_v4(),
    };
    let profile_id = identity.profile.id;
    let harness = Harness::new(ApiBehavior::default(), Some(identity), Some("token"), false);

    let restored = harness.service.restore().await.expect("restore");
    assert!(matches!(restored, AuthState::Authenticated(_)));
    assert_eq!(
        harness.calls(),
        ["active_profile", "credentials_load", "me", "identity_load"]
    );

    harness.calls.lock().expect("call log").clear();
    let switched = harness
        .service
        .switch_profile(profile_id)
        .await
        .expect("switch profile");
    assert!(matches!(switched, AuthState::Authenticated(_)));
    assert_eq!(harness.calls().first(), Some(&"profile_activate"));
}

#[tokio::test]
async fn sqlite_profile_identity_is_transactional_and_switchable() {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("profiles.sqlite3");
    let store = SqliteStore::open(&path).await.expect("store");
    let first = test_identity(test_profile());
    store
        .save_profile_identity(&first)
        .await
        .expect("first identity");

    let mut second_profile = test_profile();
    second_profile.origin = String::from("https://second.example.test/");
    let second = test_identity(second_profile);
    store
        .save_profile_identity(&second)
        .await
        .expect("second identity");
    assert_eq!(
        store.active_profile().await.expect("active profile"),
        Some(second.profile.clone())
    );

    store
        .activate_profile(first.profile.id)
        .await
        .expect("activate first");
    let restored = store
        .profile_identity(first.profile.id, first.user.id)
        .await
        .expect("first identity")
        .expect("stored identity");
    assert!(restored.profile.active);
    assert_eq!(restored.device_id, first.device_id);
}

fn test_identity(profile: Profile) -> ProfileIdentity {
    ProfileIdentity {
        profile,
        user: test_user(),
        device_id: Uuid::new_v4(),
    }
}

fn test_profile() -> Profile {
    let now = Utc::now();
    Profile {
        id: Uuid::new_v4(),
        origin: String::from("https://example.test/wisecrow/"),
        imported_ca_fingerprint: None,
        active: true,
        created_at: now,
        updated_at: now,
    }
}

fn test_user() -> UserDto {
    UserDto {
        id: 7,
        display_name: String::from("Test User"),
    }
}
