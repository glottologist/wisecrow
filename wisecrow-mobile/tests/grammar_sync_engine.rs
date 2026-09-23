//! The grammar phases of the sync engine.
//!
//! Three properties matter here. The outbox is handed over before anything is
//! downloaded, so the mastery the device then pulls already accounts for the
//! answers it took offline. A server that does not speak version 2 costs the
//! device nothing but grammar. And a failed upload leaves the outbox exactly
//! as it was, because an answer the server never heard is an answer the device
//! still owes it.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wisecrow_dto::{
    CardChangePageDto, CardChangeRequestDto, CorpusChangePageDto, CorpusChangeRequestDto,
    CorpusPageDto, CorpusSnapshotRequestDto, DeviceRegistrationRequestDto,
    GrammarAttemptBatchRequestDto, GrammarAttemptBatchResponseDto, GrammarBankChangeDto,
    GrammarBankChangePageDto, GrammarBankChangeRequestDto, GrammarChangeOperationDto,
    GrammarMasteryChangeDto, GrammarMasteryChangePageDto, GrammarMasteryChangeRequestDto,
    GrammarMasteryStateDto, LanguageInfo, LanguagePairDto, MobileCapabilitiesDto, MobileFeatureDto,
    MobileSessionDto, NbackBatchRequestDto, NbackBatchResponseDto, OfflineAttemptStatusDto,
    OfflineGrammarItemDto, RegisteredDeviceDto, ReviewBatchRequestDto, ReviewBatchResponseDto,
    ScriptDirection, UserDto, VerdictDto, MOBILE_PROTOCOL_VERSION, MOBILE_PROTOCOL_VERSION_V2,
};
use wisecrow_mobile::application::{
    CorpusRepository, GrammarRepository, MobileApi, MobileError, ProfileRepository, QueuedAttempt,
};
use wisecrow_mobile::storage::models::{Profile, ProfileIdentity};
use wisecrow_mobile::storage::SqliteStore;
use wisecrow_mobile::sync::{SyncEngine, SyncOutcome, SyncReason};

/// What the fake server does when the outbox arrives.
#[derive(Clone, Copy, PartialEq, Eq)]
enum UploadBehaviour {
    Accept,
    Fail,
}

struct ScriptedV2 {
    speaks_v2: bool,
    upload: UploadBehaviour,
    calls: Mutex<Vec<&'static str>>,
}

impl ScriptedV2 {
    fn new(speaks_v2: bool, upload: UploadBehaviour) -> Self {
        Self {
            speaks_v2,
            upload,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, call: &'static str) {
        self.calls.lock().expect("calls").push(call);
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().expect("calls").clone()
    }
}

fn v1_capabilities() -> MobileCapabilitiesDto {
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

fn item() -> OfflineGrammarItemDto {
    OfflineGrammarItemDto {
        item_id: 1,
        revision: 1,
        rule_id: 10,
        rule_slug: String::from("ser-vs-estar"),
        rule_title: String::from("Ser vs estar"),
        rule_explanation: String::from("Permanent against temporary."),
        level: String::from("A1"),
        language: String::from("es"),
        prompt: String::from("Yo ___ cansado."),
        hint: None,
        options: Vec::new(),
        answer: Some(String::from("estoy")),
        accepted: Vec::new(),
        correct_option: None,
    }
}

#[async_trait]
impl MobileApi for ScriptedV2 {
    async fn capabilities(&self) -> Result<MobileCapabilitiesDto, MobileError> {
        Ok(v1_capabilities())
    }

    async fn capabilities_v2(&self) -> Result<Option<MobileCapabilitiesDto>, MobileError> {
        self.record("capabilities_v2");
        if !self.speaks_v2 {
            return Ok(None);
        }
        Ok(Some(MobileCapabilitiesDto {
            protocol_version: MOBILE_PROTOCOL_VERSION_V2,
            supported_features: vec![
                MobileFeatureDto::CorpusSync,
                MobileFeatureDto::GrammarBankSync,
                MobileFeatureDto::GrammarMasterySync,
                MobileFeatureDto::GrammarAttemptUpload,
            ],
            ..v1_capabilities()
        }))
    }

    async fn me(&self) -> Result<UserDto, MobileError> {
        Ok(UserDto {
            id: 7,
            display_name: String::from("Test User"),
        })
    }

    async fn languages(&self) -> Result<Vec<LanguageInfo>, MobileError> {
        Ok(vec![LanguageInfo {
            code: String::from("es"),
            name: String::from("Spanish"),
            script_direction: ScriptDirection::Ltr,
        }])
    }

    async fn upload_reviews(
        &self,
        request: &ReviewBatchRequestDto,
    ) -> Result<ReviewBatchResponseDto, MobileError> {
        assert!(request.events.is_empty(), "nothing vocabulary is pending");
        Ok(ReviewBatchResponseDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            acknowledgements: Vec::new(),
            cards: Vec::new(),
        })
    }

    async fn upload_nback(
        &self,
        _request: &NbackBatchRequestDto,
    ) -> Result<NbackBatchResponseDto, MobileError> {
        Ok(NbackBatchResponseDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            acknowledgements: Vec::new(),
        })
    }

    async fn card_changes(
        &self,
        request: &CardChangeRequestDto,
    ) -> Result<CardChangePageDto, MobileError> {
        Ok(CardChangePageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            changes: Vec::new(),
            next_cursor: request.cursor,
            has_more: false,
            change_watermark: request.cursor,
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
        request: &CorpusChangeRequestDto,
    ) -> Result<CorpusChangePageDto, MobileError> {
        Ok(CorpusChangePageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            pair: request.pair.clone(),
            changes: Vec::new(),
            next_cursor: request.cursor,
            has_more: false,
            change_watermark: request.cursor,
        })
    }

    async fn grammar_bank_changes(
        &self,
        request: &GrammarBankChangeRequestDto,
    ) -> Result<GrammarBankChangePageDto, MobileError> {
        self.record("grammar_bank_changes");
        assert_eq!(request.protocol_version, MOBILE_PROTOCOL_VERSION_V2);
        assert_eq!(request.language, "es");
        Ok(GrammarBankChangePageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION_V2,
            language: String::from("es"),
            changes: if request.cursor == 0 {
                vec![GrammarBankChangeDto {
                    sequence: 5,
                    item_id: 1,
                    operation: GrammarChangeOperationDto::Upsert,
                    item: Some(item()),
                }]
            } else {
                Vec::new()
            },
            next_cursor: 5,
            has_more: false,
        })
    }

    async fn grammar_mastery_changes(
        &self,
        request: &GrammarMasteryChangeRequestDto,
    ) -> Result<GrammarMasteryChangePageDto, MobileError> {
        self.record("grammar_mastery_changes");
        let due = Utc::now();
        Ok(GrammarMasteryChangePageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION_V2,
            changes: if request.cursor == 0 {
                vec![GrammarMasteryChangeDto {
                    sequence: 8,
                    rule_id: 10,
                    operation: GrammarChangeOperationDto::Upsert,
                    mastery: Some(GrammarMasteryStateDto {
                        rule_id: 10,
                        rule_slug: String::from("ser-vs-estar"),
                        stability: 2.0,
                        difficulty: 5.0,
                        elapsed_days: 0,
                        scheduled_days: 1,
                        reps: 1,
                        lapses: 0,
                        state: 1,
                        accuracy: Some(1.0),
                        attempts: 1,
                        last_review: Some(due),
                        due,
                    }),
                }]
            } else {
                Vec::new()
            },
            next_cursor: 8,
            has_more: false,
        })
    }

    async fn upload_grammar_attempts(
        &self,
        request: &GrammarAttemptBatchRequestDto,
    ) -> Result<GrammarAttemptBatchResponseDto, MobileError> {
        self.record("upload_grammar_attempts");
        if self.upload == UploadBehaviour::Fail {
            return Err(MobileError::Retryable);
        }
        Ok(GrammarAttemptBatchResponseDto {
            protocol_version: MOBILE_PROTOCOL_VERSION_V2,
            results: request
                .attempts
                .iter()
                .map(|attempt| {
                    OfflineAttemptStatusDto::Accepted(VerdictDto {
                        event_id: attempt.event_id,
                        correct: true,
                    })
                })
                .collect(),
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

fn identity() -> ProfileIdentity {
    let now = Utc::now();
    ProfileIdentity {
        profile: Profile {
            id: Uuid::new_v4(),
            origin: String::from("https://grammar-sync.example.test/"),
            imported_ca_fingerprint: None,
            active: true,
            created_at: now,
            updated_at: now,
        },
        user: UserDto {
            id: 7,
            display_name: String::from("Test User"),
        },
        device_id: Uuid::new_v4(),
    }
}

fn pair() -> LanguagePairDto {
    LanguagePairDto {
        native_lang: String::from("en"),
        foreign_lang: String::from("es"),
    }
}

fn queued() -> QueuedAttempt {
    QueuedAttempt {
        event_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        item_id: 1,
        revision: 1,
        answer: String::from("estoy"),
        chose_option: false,
        hint_shown: false,
        ordinal: 1,
        occurred_at: Utc::now(),
        language: String::from("es"),
        level: Some(String::from("A1")),
    }
}

/// A ready pair is what tells the device it is studying a language, so the
/// store has to look as though vocabulary has already arrived.
async fn seeded_store(root: &std::path::Path) -> Arc<SqliteStore> {
    let store = Arc::new(
        SqliteStore::open(&root.join("grammar-sync.sqlite3"))
            .await
            .expect("store"),
    );
    store
        .save_profile_identity(&identity())
        .await
        .expect("identity");
    store
        .begin_snapshot(&pair(), 1, Some(100))
        .await
        .expect("begin snapshot");
    store
        .apply_snapshot_page(&CorpusPageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            pair: pair(),
            translations: Vec::new(),
            next_cursor: 0,
            has_more: false,
            snapshot_watermark: 1,
        })
        .await
        .expect("snapshot");
    store
}

async fn run(store: Arc<SqliteStore>, api: Arc<ScriptedV2>) -> SyncOutcome {
    let store_resource: Arc<dyn wisecrow_mobile::application::LocalStore> = store;
    let api_resource: Arc<dyn MobileApi> = api;
    SyncEngine::new(
        store_resource,
        api_resource,
        Arc::new(CancellationToken::new()),
    )
    .run(SyncReason::Launch)
    .await
}

#[tokio::test]
async fn the_outbox_is_handed_over_before_the_mastery_it_changes_is_pulled() {
    let directory = tempdir().expect("temporary directory");
    let store = seeded_store(directory.path()).await;
    store.queue_attempt(&queued()).await.expect("queue");

    let api = Arc::new(ScriptedV2::new(true, UploadBehaviour::Accept));
    let outcome = run(Arc::clone(&store), Arc::clone(&api)).await;
    assert!(
        matches!(outcome, SyncOutcome::Complete { .. }),
        "{outcome:?}"
    );

    let calls = api.calls();
    let upload = calls
        .iter()
        .position(|call| *call == "upload_grammar_attempts")
        .expect("the outbox was offered");
    let mastery = calls
        .iter()
        .position(|call| *call == "grammar_mastery_changes")
        .expect("mastery was pulled");
    assert!(
        upload < mastery,
        "mastery pulled before the answers that move it would arrive stale: {calls:?}"
    );

    assert!(
        store
            .pending_attempts(10)
            .await
            .expect("pending")
            .is_empty(),
        "an acknowledged answer leaves the outbox"
    );
    assert_eq!(store.grammar_items("es").await.expect("items").len(), 1);
    assert_eq!(store.grammar_mastery("es").await.expect("mastery").len(), 1);
    let cursors = store.grammar_cursors("es").await.expect("cursors");
    assert_eq!(cursors.bank, 5);
    assert_eq!(cursors.mastery, 8);
}

/// An answer the server never heard is one the device still owes it.
#[tokio::test]
async fn a_failed_upload_leaves_the_outbox_intact() {
    let directory = tempdir().expect("temporary directory");
    let store = seeded_store(directory.path()).await;
    let attempt = queued();
    store.queue_attempt(&attempt).await.expect("queue");

    let api = Arc::new(ScriptedV2::new(true, UploadBehaviour::Fail));
    let outcome = run(Arc::clone(&store), api).await;
    assert!(matches!(outcome, SyncOutcome::Retryable), "{outcome:?}");

    let pending = store.pending_attempts(10).await.expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].event_id, attempt.event_id);
}

/// A server that speaks only version 1 costs the device grammar, and nothing
/// else.
#[tokio::test]
async fn a_version_one_server_leaves_vocabulary_working() {
    let directory = tempdir().expect("temporary directory");
    let store = seeded_store(directory.path()).await;
    store.queue_attempt(&queued()).await.expect("queue");

    let api = Arc::new(ScriptedV2::new(false, UploadBehaviour::Accept));
    let outcome = run(Arc::clone(&store), Arc::clone(&api)).await;
    assert!(
        matches!(outcome, SyncOutcome::Complete { .. }),
        "{outcome:?}"
    );

    assert_eq!(
        api.calls(),
        vec!["capabilities_v2"],
        "no grammar endpoint is called against a version-1 server"
    );
    assert_eq!(
        store.pending_attempts(10).await.expect("pending").len(),
        1,
        "the answers wait for a server that can take them"
    );
}
