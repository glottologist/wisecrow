use std::{
    collections::HashSet,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use async_trait::async_trait;
use chrono::{Duration, TimeZone, Utc};
use tempfile::tempdir;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wisecrow_dto::{
    CardChangeDto, CardChangePageDto, CardChangeRequestDto, CardSnapshotDto, CardStatusDto,
    CorpusChangeDto, CorpusChangeKindDto, CorpusChangePageDto, CorpusChangeRequestDto,
    CorpusPageDto, CorpusSnapshotRequestDto, CorpusTranslationDto, DeviceRegistrationRequestDto,
    LanguageInfo, LanguagePairDto, MobileCapabilitiesDto, MobileFeatureDto, MobileSessionDto,
    NbackBatchRequestDto, NbackBatchResponseDto, NbackModeDto, NbackSessionUploadDto,
    NbackUploadAckDto, NbackUploadStatusDto, RegisteredDeviceDto, ReviewBatchRequestDto,
    ReviewBatchResponseDto, ReviewEventAckDto, ReviewEventStatusDto, ReviewRatingDto,
    ScriptDirection, UserDto, MOBILE_PROTOCOL_VERSION,
};
use wisecrow_mobile::{
    application::{
        AppServices, BackgroundScheduler, ContentRepository, CorpusRepository, CredentialStore,
        LearningRepository, MobileApi, MobileError, ProfileRepository,
    },
    auth::AuthState,
    storage::{
        models::{LocalAnswer, LocalSessionRequest, Profile, ProfileIdentity},
        SqliteStore,
    },
    sync::{SyncEngine, SyncOutcome, SyncReason},
};

#[derive(Debug, PartialEq, Eq)]
struct DatabaseState {
    translations: Vec<(i32, String)>,
    card_cursor: i64,
    snapshot_cursor: i32,
    change_cursor: i64,
    pending_reviews: i64,
    pending_nback: i64,
    language_count: i64,
    last_success: bool,
}

#[tokio::test]
async fn every_committed_response_boundary_resumes_to_identical_state() {
    let expected = run_restart_scenario(None).await;
    for boundary in 0..=5 {
        assert_eq!(run_restart_scenario(Some(boundary)).await, expected);
    }
}

#[tokio::test]
async fn permanent_rejections_remain_visible_without_blocking_sync() {
    let directory = tempdir().expect("temporary directory");
    let store = open_seeded_store(directory.path()).await;
    let api = Arc::new(ScriptedApi::new(None, true, false));
    let outcome = run_engine(store.clone(), api).await;
    assert_eq!(
        outcome,
        SyncOutcome::Complete {
            media_prefetch_limit: 25
        }
    );
    let status: String = sqlx::query_scalar("SELECT status FROM review_outbox")
        .fetch_one(store.pool())
        .await
        .expect("review status");
    assert_eq!(status, "rejected");
    assert!(store.pending_reviews(10).await.expect("pending").is_empty());
}

#[tokio::test]
async fn authentication_expiry_stops_calls_and_preserves_offline_data() {
    let directory = tempdir().expect("temporary directory");
    let store = open_seeded_store(directory.path()).await;
    let api = Arc::new(ScriptedApi::new(None, false, true));
    let credentials = Arc::new(FakeCredentials::new("token"));
    let background = Arc::new(FakeBackground);
    let store_resource: Arc<dyn wisecrow_mobile::application::LocalStore> = store.clone();
    let api_resource: Arc<dyn MobileApi> = api.clone();
    let credential_resource: Arc<dyn CredentialStore> = credentials.clone();
    let background_resource: Arc<dyn BackgroundScheduler> = background;
    let services = AppServices::new(
        store_resource,
        api_resource,
        credential_resource,
        background_resource,
    );
    let mut state = AuthState::Authenticated(identity());
    let outcome = services
        .sync(
            SyncReason::Launch,
            Arc::new(CancellationToken::new()),
            &mut state,
        )
        .await;
    assert_eq!(outcome, SyncOutcome::AuthenticationExpired);
    assert_eq!(state, AuthState::Anonymous);
    assert!(!credentials.has_token());
    assert_eq!(api.calls(), vec!["me"]);
    let translation_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM translations")
        .fetch_one(store.pool())
        .await
        .expect("translation count");
    assert_eq!(translation_count, 0);
    let pair_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM language_pairs")
        .fetch_one(store.pool())
        .await
        .expect("pair count");
    assert_eq!(pair_count, 1);
}

#[tokio::test]
async fn a_second_sync_for_the_same_profile_returns_already_running() {
    let directory = tempdir().expect("temporary directory");
    let store = open_seeded_store(directory.path()).await;
    let api = Arc::new(ScriptedApi::new_blocking());
    let first = sync_engine(
        store.clone(),
        api.clone(),
        Arc::new(CancellationToken::new()),
    );
    let first_run = tokio::spawn(async move { first.run(SyncReason::Launch).await });
    api.wait_until_blocked().await;
    let second = sync_engine(store, api.clone(), Arc::new(CancellationToken::new()));
    assert_eq!(
        second.run(SyncReason::Manual).await,
        SyncOutcome::AlreadyRunning
    );
    api.release();
    assert_eq!(
        first_run.await.expect("first sync task"),
        SyncOutcome::Complete {
            media_prefetch_limit: 25
        }
    );
}

#[tokio::test]
async fn cancellation_stops_before_the_first_network_call() {
    let directory = tempdir().expect("temporary directory");
    let store = open_seeded_store(directory.path()).await;
    let api = Arc::new(ScriptedApi::new(None, false, false));
    let cancellation = Arc::new(CancellationToken::new());
    cancellation.cancel();
    let engine = sync_engine(store, api.clone(), cancellation);
    assert_eq!(
        engine.run(SyncReason::Periodic).await,
        SyncOutcome::Cancelled
    );
    assert!(api.calls().is_empty());
}

async fn run_restart_scenario(fail_after: Option<usize>) -> DatabaseState {
    let directory = tempdir().expect("temporary directory");
    let store = open_seeded_store(directory.path()).await;
    let api = Arc::new(ScriptedApi::new(fail_after, false, false));
    for _ in 0..8 {
        match run_engine(store.clone(), api.clone()).await {
            SyncOutcome::Complete {
                media_prefetch_limit,
            } => {
                assert_eq!(media_prefetch_limit, 25);
                assert!(api.unique_data_bodies());
                return database_state(&store).await;
            }
            SyncOutcome::Retryable => {}
            outcome => panic!("unexpected sync outcome: {outcome:?}"),
        }
    }
    panic!("sync did not complete within the bounded restart count")
}

async fn run_engine(store: Arc<SqliteStore>, api: Arc<ScriptedApi>) -> SyncOutcome {
    sync_engine(store, api, Arc::new(CancellationToken::new()))
        .run(SyncReason::Launch)
        .await
}

fn sync_engine(
    store: Arc<SqliteStore>,
    api: Arc<ScriptedApi>,
    cancellation: Arc<CancellationToken>,
) -> SyncEngine {
    let store_resource: Arc<dyn wisecrow_mobile::application::LocalStore> = store;
    let api_resource: Arc<dyn MobileApi> = api;
    SyncEngine::new(store_resource, api_resource, cancellation)
}

async fn open_seeded_store(root: &std::path::Path) -> Arc<SqliteStore> {
    let store = Arc::new(
        SqliteStore::open(&root.join("sync.sqlite3"))
            .await
            .expect("store"),
    );
    let identity = identity();
    store
        .save_profile_identity(&identity)
        .await
        .expect("identity");
    seed_pending_progress(&store, &identity).await;
    store
        .begin_snapshot(&pair(), 100, Some(10_000))
        .await
        .expect("begin replacement snapshot");
    store
}

async fn seed_pending_progress(store: &SqliteStore, identity: &ProfileIdentity) {
    let pair = pair();
    store
        .begin_snapshot(&pair, 1, Some(100))
        .await
        .expect("begin seed snapshot");
    store
        .apply_snapshot_page(&snapshot_page(1, 1, false))
        .await
        .expect("seed snapshot");
    store
        .apply_card_page(&card_page(1, 1))
        .await
        .expect("seed card");
    let started_at = timestamp();
    let session = store
        .create_session(&LocalSessionRequest {
            id: Uuid::from_u128(10),
            pair: pair.clone(),
            deck_size: 1,
            speed_ms: 2_000,
            started_at,
        })
        .await
        .expect("learning session");
    store
        .answer(&LocalAnswer {
            session_id: session.id,
            event_id: Uuid::from_u128(11),
            device_id: identity.device_id,
            translation_id: 1,
            rating: ReviewRatingDto::Good,
            occurred_at: started_at + Duration::seconds(1),
        })
        .await
        .expect("local answer");
    store.save_nback(&nback(&pair)).await.expect("save N-Back");
}

async fn database_state(store: &SqliteStore) -> DatabaseState {
    DatabaseState {
        translations: sqlx::query_as(
            "SELECT translation_id, from_phrase FROM translations ORDER BY translation_id",
        )
        .fetch_all(store.pool())
        .await
        .expect("translations"),
        card_cursor: scalar(store, "SELECT card_cursor FROM sync_state").await,
        snapshot_cursor: scalar(store, "SELECT snapshot_after_id FROM language_pairs").await,
        change_cursor: scalar(store, "SELECT change_cursor FROM language_pairs").await,
        pending_reviews: scalar(
            store,
            "SELECT COUNT(*) FROM review_outbox WHERE status = 'pending'",
        )
        .await,
        pending_nback: scalar(
            store,
            "SELECT COUNT(*) FROM nback_outbox WHERE status = 'pending'",
        )
        .await,
        language_count: scalar(store, "SELECT COUNT(*) FROM languages").await,
        last_success: sqlx::query_scalar::<_, Option<String>>(
            "SELECT last_success_at FROM sync_state",
        )
        .fetch_one(store.pool())
        .await
        .expect("last success")
        .is_some(),
    }
}

async fn scalar<T>(store: &SqliteStore, query: &str) -> T
where
    T: for<'row> sqlx::Decode<'row, sqlx::Sqlite> + sqlx::Type<sqlx::Sqlite> + Send + Unpin,
{
    sqlx::query_scalar(query)
        .fetch_one(store.pool())
        .await
        .expect("scalar")
}

struct ScriptedApi {
    fail_after: Option<usize>,
    failure_fired: AtomicBool,
    successful_responses: AtomicUsize,
    reject_reviews: bool,
    unauthorized: bool,
    bodies: Mutex<HashSet<String>>,
    calls: Mutex<Vec<&'static str>>,
    block_me: bool,
    me_entered: Notify,
    me_release: Notify,
}

impl ScriptedApi {
    fn new(fail_after: Option<usize>, reject_reviews: bool, unauthorized: bool) -> Self {
        Self {
            fail_after,
            failure_fired: AtomicBool::new(false),
            successful_responses: AtomicUsize::new(0),
            reject_reviews,
            unauthorized,
            bodies: Mutex::new(HashSet::new()),
            calls: Mutex::new(Vec::new()),
            block_me: false,
            me_entered: Notify::new(),
            me_release: Notify::new(),
        }
    }

    fn new_blocking() -> Self {
        Self {
            block_me: true,
            ..Self::new(None, false, false)
        }
    }

    async fn wait_until_blocked(&self) {
        self.me_entered.notified().await;
    }

    fn release(&self) {
        self.me_release.notify_one();
    }

    fn begin_data_call<T: serde::Serialize>(&self, request: &T) -> Result<(), MobileError> {
        if self.fail_after == Some(self.successful_responses.load(Ordering::SeqCst))
            && !self.failure_fired.swap(true, Ordering::SeqCst)
        {
            return Err(MobileError::Retryable);
        }
        let body = serde_json::to_string(request).expect("serialize request");
        assert!(self.bodies.lock().expect("bodies").insert(body));
        Ok(())
    }

    fn finish_data_call(&self) {
        self.successful_responses.fetch_add(1, Ordering::SeqCst);
    }

    fn unique_data_bodies(&self) -> bool {
        self.bodies.lock().expect("bodies").len()
            == self.successful_responses.load(Ordering::SeqCst)
    }

    fn record(&self, call: &'static str) {
        self.calls.lock().expect("calls").push(call);
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().expect("calls").clone()
    }
}

#[async_trait]
impl MobileApi for ScriptedApi {
    async fn capabilities(&self) -> Result<MobileCapabilitiesDto, MobileError> {
        self.record("capabilities");
        Ok(capabilities())
    }

    async fn languages(&self) -> Result<Vec<LanguageInfo>, MobileError> {
        self.record("languages");
        Ok(languages())
    }

    async fn me(&self) -> Result<UserDto, MobileError> {
        self.record("me");
        if self.block_me {
            self.me_entered.notify_one();
            self.me_release.notified().await;
        }
        if self.unauthorized {
            Err(MobileError::Authentication)
        } else {
            Ok(identity().user)
        }
    }

    async fn upload_reviews(
        &self,
        request: &ReviewBatchRequestDto,
    ) -> Result<ReviewBatchResponseDto, MobileError> {
        self.begin_data_call(request)?;
        let acknowledgements = request
            .events
            .iter()
            .map(|event| ReviewEventAckDto {
                event_id: event.event_id,
                status: if self.reject_reviews {
                    ReviewEventStatusDto::Rejected {
                        reason: String::from("invalid event"),
                    }
                } else {
                    ReviewEventStatusDto::Applied
                },
            })
            .collect();
        self.finish_data_call();
        Ok(ReviewBatchResponseDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            acknowledgements,
            cards: if self.reject_reviews {
                Vec::new()
            } else {
                vec![card(1, 1)]
            },
        })
    }

    async fn upload_nback(
        &self,
        request: &NbackBatchRequestDto,
    ) -> Result<NbackBatchResponseDto, MobileError> {
        self.begin_data_call(request)?;
        let acknowledgements = request
            .sessions
            .iter()
            .map(|session| NbackUploadAckDto {
                client_session_id: session.client_session_id,
                status: NbackUploadStatusDto::Applied,
                result: None,
            })
            .collect();
        self.finish_data_call();
        Ok(NbackBatchResponseDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            acknowledgements,
        })
    }

    async fn card_changes(
        &self,
        request: &CardChangeRequestDto,
    ) -> Result<CardChangePageDto, MobileError> {
        self.begin_data_call(request)?;
        assert_eq!(request.cursor, 1);
        self.finish_data_call();
        Ok(card_page(2, 2))
    }

    async fn corpus_snapshot(
        &self,
        request: &CorpusSnapshotRequestDto,
    ) -> Result<CorpusPageDto, MobileError> {
        self.begin_data_call(request)?;
        assert_eq!(request.snapshot_watermark, Some(100));
        let page = if request.after_translation_id == 0 {
            snapshot_page(1, 25, true)
        } else {
            assert_eq!(request.after_translation_id, 25);
            snapshot_page(26, 50, false)
        };
        self.finish_data_call();
        Ok(page)
    }

    async fn corpus_changes(
        &self,
        request: &CorpusChangeRequestDto,
    ) -> Result<CorpusChangePageDto, MobileError> {
        self.begin_data_call(request)?;
        assert_eq!(request.cursor, 100);
        self.finish_data_call();
        Ok(CorpusChangePageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            pair: pair(),
            changes: vec![CorpusChangeDto {
                sequence: 101,
                changed_at: timestamp(),
                kind: CorpusChangeKindDto::Upsert,
                translation_id: 50,
                translation: Some(translation(50, "changed-50")),
            }],
            next_cursor: 101,
            has_more: false,
            change_watermark: 101,
        })
    }

    async fn login(&self, _email: &str, _password: &str) -> Result<MobileSessionDto, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn logout(&self) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn register_device(
        &self,
        _request: &DeviceRegistrationRequestDto,
    ) -> Result<RegisteredDeviceDto, MobileError> {
        Err(MobileError::Unsupported)
    }
}

struct FakeCredentials {
    token: Mutex<Option<String>>,
}

impl FakeCredentials {
    fn new(token: &str) -> Self {
        Self {
            token: Mutex::new(Some(String::from(token))),
        }
    }

    fn has_token(&self) -> bool {
        self.token.lock().expect("token").is_some()
    }
}

#[async_trait]
impl CredentialStore for FakeCredentials {
    async fn load(&self, _profile_id: Uuid) -> Result<Option<String>, MobileError> {
        Ok(self.token.lock().expect("token").clone())
    }

    async fn save(&self, _profile_id: Uuid, token: &str) -> Result<(), MobileError> {
        *self.token.lock().expect("token") = Some(String::from(token));
        Ok(())
    }

    async fn delete(&self, _profile_id: Uuid) -> Result<(), MobileError> {
        self.token.lock().expect("token").take();
        Ok(())
    }
}

struct FakeBackground;

#[async_trait]
impl BackgroundScheduler for FakeBackground {
    async fn schedule_sync(&self, _profile_id: Uuid) -> Result<(), MobileError> {
        Ok(())
    }

    async fn cancel_sync(&self, _profile_id: Uuid) -> Result<(), MobileError> {
        Ok(())
    }
}

fn capabilities() -> MobileCapabilitiesDto {
    MobileCapabilitiesDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        supported_features: vec![
            MobileFeatureDto::CorpusSync,
            MobileFeatureDto::CardSync,
            MobileFeatureDto::ReviewUpload,
            MobileFeatureDto::NbackUpload,
        ],
        max_snapshot_page: 25,
        max_review_batch: 10,
        max_nback_batch: 10,
        server_version: String::from("test"),
    }
}

fn languages() -> Vec<LanguageInfo> {
    vec![
        LanguageInfo {
            code: String::from("en"),
            name: String::from("English"),
            script_direction: ScriptDirection::Ltr,
        },
        LanguageInfo {
            code: String::from("de"),
            name: String::from("German"),
            script_direction: ScriptDirection::Ltr,
        },
    ]
}

fn snapshot_page(first: i32, last: i32, has_more: bool) -> CorpusPageDto {
    CorpusPageDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        pair: pair(),
        translations: (first..=last)
            .map(|translation_id| translation(translation_id, "snapshot"))
            .collect(),
        next_cursor: last,
        has_more,
        snapshot_watermark: if last == 1 { 1 } else { 100 },
    }
}

fn translation(translation_id: i32, prefix: &str) -> CorpusTranslationDto {
    CorpusTranslationDto {
        translation_id,
        from_phrase: format!("{prefix}-{translation_id}"),
        to_phrase: format!("to-{translation_id}"),
        frequency: 100 - translation_id,
        is_phrase: translation_id % 5 == 0,
    }
}

fn card_page(sequence: i64, translation_id: i32) -> CardChangePageDto {
    CardChangePageDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        changes: vec![CardChangeDto::Upsert {
            sequence,
            card: card(translation_id, sequence),
        }],
        next_cursor: sequence,
        has_more: false,
        change_watermark: sequence,
    }
}

fn card(translation_id: i32, cursor: i64) -> CardSnapshotDto {
    CardSnapshotDto {
        translation_id,
        stability: 1.0,
        difficulty: 5.0,
        elapsed_days: 1,
        scheduled_days: 1,
        reps: 1,
        lapses: 0,
        state: CardStatusDto::Review,
        last_review: Some(timestamp() - Duration::days(1)),
        due: timestamp() - Duration::seconds(1),
        server_cursor: cursor,
    }
}

fn nback(pair: &LanguagePairDto) -> NbackSessionUploadDto {
    NbackSessionUploadDto {
        client_session_id: Uuid::from_u128(20),
        pair: pair.clone(),
        mode: NbackModeDto::AudioWritten,
        n_level: 2,
        interval_ms: 2_000,
        seed: 42,
        vocabulary_translation_ids: vec![1],
        responses: Vec::new(),
        started_at: timestamp(),
        completed_at: timestamp() + Duration::minutes(1),
    }
}

fn identity() -> ProfileIdentity {
    let now = timestamp();
    ProfileIdentity {
        profile: Profile {
            id: Uuid::from_u128(1),
            origin: String::from("https://sync.example.test/"),
            imported_ca_fingerprint: None,
            active: true,
            created_at: now,
            updated_at: now,
        },
        user: UserDto {
            id: 7,
            display_name: String::from("Learner"),
        },
        device_id: Uuid::from_u128(2),
    }
}

fn pair() -> LanguagePairDto {
    LanguagePairDto {
        native_lang: String::from("en"),
        foreign_lang: String::from("de"),
    }
}

fn timestamp() -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000, 0)
        .single()
        .expect("timestamp")
}
