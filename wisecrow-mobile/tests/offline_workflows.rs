use chrono::{Duration, TimeZone, Utc};
use tempfile::tempdir;
use uuid::Uuid;
use wisecrow_dto::{
    CachedQuizDto, CardChangeDto, CardChangePageDto, CardSnapshotDto, CardStatusDto, ClozeQuizDto,
    CorpusPageDto, CorpusTranslationDto, LanguagePairDto, NbackModeDto, NbackSessionUploadDto,
    NbackTrialResponseDto, QuizCacheKeyDto, QuizItemDto, QuizKindDto, ReviewRatingDto, UserDto,
    MOBILE_PROTOCOL_VERSION,
};
use wisecrow_mobile::application::{
    ContentRepository, CorpusRepository, LearningRepository, ProfileRepository,
};
use wisecrow_mobile::storage::models::{
    LocalAnswer, LocalSessionRequest, MediaRegistration, MediaType, Profile, ProfileIdentity,
};
use wisecrow_mobile::storage::SqliteStore;

#[tokio::test]
async fn learning_and_content_work_without_an_api_client() {
    let directory = tempdir().expect("temporary directory");
    let store = SqliteStore::open(&directory.path().join("offline.sqlite3"))
        .await
        .expect("store");
    let identity = identity();
    store
        .save_profile_identity(&identity)
        .await
        .expect("identity");
    let pair = pair();
    let started_at = Utc
        .timestamp_opt(1_800_000_000, 0)
        .single()
        .expect("timestamp");
    seed_corpus(&store, &pair).await;
    seed_cards(&store, started_at).await;

    assert_learning_is_durable(&store, &identity, &pair, started_at).await;
    assert_fast_deck_is_read_only(&store, &pair).await;
    assert_content_is_durable(&store, &pair, started_at, directory.path()).await;
}

async fn assert_learning_is_durable(
    store: &SqliteStore,
    identity: &ProfileIdentity,
    pair: &LanguagePairDto,
    started_at: chrono::DateTime<Utc>,
) {
    let request = LocalSessionRequest {
        id: Uuid::from_u128(100),
        pair: pair.clone(),
        deck_size: 10,
        speed_ms: 2_000,
        started_at,
    };
    let session = store.create_session(&request).await.expect("session");
    assert_eq!(
        store.create_session(&request).await.expect("session retry"),
        session
    );
    assert_eq!(
        session
            .cards
            .iter()
            .take(4)
            .map(|item| item.translation.translation_id)
            .collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    for (index, item) in session.cards.iter().take(4).enumerate() {
        let answer = answer(identity, &session, item.translation.translation_id, index);
        store.answer(&answer).await.expect("answer");
        if index == 0 {
            store.answer(&answer).await.expect("idempotent answer");
        }
    }
    let pending = store.pending_reviews(50).await.expect("pending reviews");
    assert_eq!(pending.len(), 4);
    let current_index: i64 = sqlx::query_scalar("SELECT current_index FROM learn_sessions")
        .fetch_one(store.pool())
        .await
        .expect("current index");
    assert_eq!(current_index, 4);
}

fn answer(
    identity: &ProfileIdentity,
    session: &wisecrow_mobile::storage::models::LocalSession,
    translation_id: i32,
    index: usize,
) -> LocalAnswer {
    LocalAnswer {
        session_id: session.id,
        event_id: Uuid::from_u128(u128::try_from(index + 1).expect("small index")),
        device_id: identity.device_id,
        translation_id,
        rating: ReviewRatingDto::Good,
        occurred_at: session.started_at
            + Duration::seconds(i64::try_from(index + 1).expect("small index")),
    }
}

async fn assert_fast_deck_is_read_only(store: &SqliteStore, pair: &LanguagePairDto) {
    let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM review_outbox")
        .fetch_one(store.pool())
        .await
        .expect("outbox count");
    let deck = store
        .ranked_translations(pair, 50)
        .await
        .expect("fast deck");
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM review_outbox")
        .fetch_one(store.pool())
        .await
        .expect("outbox count");
    assert_eq!(deck.len(), 50);
    assert!(deck
        .iter()
        .enumerate()
        .all(|(index, item)| item.is_phrase == (index % 5 == 4)));
    assert_eq!(after, before);
}

async fn assert_content_is_durable(
    store: &SqliteStore,
    pair: &LanguagePairDto,
    started_at: chrono::DateTime<Utc>,
    temporary_root: &std::path::Path,
) {
    let nback = nback(pair, started_at);
    store.save_nback(&nback).await.expect("save nback");
    assert_eq!(
        store.pending_nback(10).await.expect("pending nback"),
        vec![nback]
    );
    let quiz = quiz(started_at);
    store.save_quiz(&quiz).await.expect("save quiz");
    assert_eq!(
        store.cached_quizzes().await.expect("cached quizzes"),
        vec![quiz]
    );
    assert_media_lru(store, started_at, temporary_root).await;
}

async fn assert_media_lru(
    store: &SqliteStore,
    started_at: chrono::DateTime<Utc>,
    temporary_root: &std::path::Path,
) {
    let media_root = temporary_root.join("media");
    std::fs::create_dir(&media_root).expect("media root");
    std::fs::write(media_root.join("one.mp3"), b"one").expect("first media");
    std::fs::write(media_root.join("two.jpg"), b"two-two").expect("second media");
    store
        .register_media(
            &media_root,
            &media(1, MediaType::Audio, "one.mp3", 3, started_at),
        )
        .await
        .expect("register first media");
    store
        .register_media(
            &media_root,
            &media(
                2,
                MediaType::Image,
                "two.jpg",
                7,
                started_at + Duration::seconds(1),
            ),
        )
        .await
        .expect("register second media");
    let entries = store.media_lru(&media_root).await.expect("media LRU");
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.translation_id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    store
        .media(
            &media_root,
            1,
            MediaType::Audio,
            started_at + Duration::seconds(2),
        )
        .await
        .expect("media access")
        .expect("media entry");
    assert_eq!(
        store
            .media_lru(&media_root)
            .await
            .expect("updated LRU")
            .first()
            .map(|entry| entry.translation_id),
        Some(2)
    );
    let candidates = store
        .eviction_candidates(&media_root, 1)
        .await
        .expect("eviction candidates");
    assert_eq!(
        candidates.first().map(|entry| entry.translation_id),
        Some(2)
    );
    let retained: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_cache")
        .fetch_one(store.pool())
        .await
        .expect("retained media rows");
    assert_eq!(retained, 2);
    let candidate = candidates.first().expect("candidate");
    std::fs::remove_file(&candidate.path).expect("platform deletion");
    store
        .confirm_media_deleted(candidate.translation_id, candidate.media_type)
        .await
        .expect("confirm deletion");
    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM media_cache")
        .fetch_one(store.pool())
        .await
        .expect("remaining media rows");
    assert_eq!(remaining, 1);
}

fn media(
    translation_id: i32,
    media_type: MediaType,
    file_name: &str,
    byte_length: u64,
    last_accessed_at: chrono::DateTime<Utc>,
) -> MediaRegistration {
    MediaRegistration {
        translation_id,
        media_type,
        file_name: String::from(file_name),
        byte_length,
        attribution: None,
        last_accessed_at,
    }
}

async fn seed_corpus(store: &SqliteStore, pair: &LanguagePairDto) {
    store
        .begin_snapshot(pair, 50, Some(10_000))
        .await
        .expect("begin snapshot");
    let translations = (1..=50)
        .map(|translation_id| CorpusTranslationDto {
            translation_id,
            from_phrase: format!("from-{translation_id}"),
            to_phrase: format!("to-{translation_id}"),
            frequency: 100 - translation_id,
            is_phrase: translation_id % 5 == 0,
        })
        .collect();
    store
        .apply_snapshot_page(&CorpusPageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            pair: pair.clone(),
            translations,
            next_cursor: 50,
            has_more: false,
            snapshot_watermark: 50,
        })
        .await
        .expect("snapshot");
}

async fn seed_cards(store: &SqliteStore, started_at: chrono::DateTime<Utc>) {
    let states = [
        CardStatusDto::Relearning,
        CardStatusDto::Learning,
        CardStatusDto::New,
        CardStatusDto::Review,
    ];
    let changes = states
        .into_iter()
        .enumerate()
        .map(|(index, state)| {
            let sequence = i64::try_from(index + 1).expect("small sequence");
            CardChangeDto::Upsert {
                sequence,
                card: card(
                    i32::try_from(index + 1).expect("small id"),
                    state,
                    started_at,
                    sequence,
                ),
            }
        })
        .collect();
    store
        .apply_card_page(&CardChangePageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION,
            changes,
            next_cursor: 4,
            has_more: false,
            change_watermark: 4,
        })
        .await
        .expect("cards");
}

fn card(
    translation_id: i32,
    state: CardStatusDto,
    started_at: chrono::DateTime<Utc>,
    cursor: i64,
) -> CardSnapshotDto {
    CardSnapshotDto {
        translation_id,
        stability: 1.0,
        difficulty: 5.0,
        elapsed_days: 1,
        scheduled_days: 1,
        reps: 1,
        lapses: 0,
        state,
        last_review: Some(started_at - Duration::days(1)),
        due: started_at - Duration::seconds(1),
        server_cursor: cursor,
    }
}

fn nback(pair: &LanguagePairDto, started_at: chrono::DateTime<Utc>) -> NbackSessionUploadDto {
    NbackSessionUploadDto {
        client_session_id: Uuid::from_u128(200),
        pair: pair.clone(),
        mode: NbackModeDto::AudioWritten,
        n_level: 2,
        interval_ms: 2_000,
        seed: 42,
        vocabulary_translation_ids: vec![1, 2, 3, 4],
        responses: vec![NbackTrialResponseDto {
            trial_number: 1,
            audio_response: Some(true),
            visual_response: Some(false),
            response_time_ms: 500,
        }],
        started_at,
        completed_at: started_at + Duration::minutes(1),
    }
}

fn quiz(created_at: chrono::DateTime<Utc>) -> CachedQuizDto {
    CachedQuizDto {
        key: QuizCacheKeyDto {
            source_sha256: String::from("abc123"),
            native_lang: String::from("en"),
            foreign_lang: String::from("de"),
            kind: QuizKindDto::Cloze,
            cefr_level: Some(String::from("A2")),
            item_count: 1,
        },
        label: String::from("Lesson"),
        items: vec![QuizItemDto::Cloze(ClozeQuizDto {
            sentence_with_blank: String::from("Ich ___ hier"),
            answer: String::from("bin"),
            hint: None,
            rule_context: None,
        })],
        created_at,
    }
}

fn identity() -> ProfileIdentity {
    let now = Utc::now();
    ProfileIdentity {
        profile: Profile {
            id: Uuid::from_u128(1),
            origin: String::from("https://offline.example.test/"),
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

// --- The grammar round trip ---------------------------------------------
//
// This is the criterion that actually demonstrates mobile completeness: an
// answer taken with no network survives the app being killed, reaches the
// server when one returns, and leaves the server holding the same state the
// device projected for itself.

mod grammar_round_trip {
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
        GrammarMasteryStateDto, LanguageInfo, LanguagePairDto, MobileCapabilitiesDto,
        MobileFeatureDto, MobileSessionDto, NbackBatchRequestDto, NbackBatchResponseDto,
        OfflineAttemptDto, OfflineAttemptStatusDto, OfflineGrammarItemDto, RegisteredDeviceDto,
        ReviewBatchRequestDto, ReviewBatchResponseDto, ScriptDirection, UserDto, VerdictDto,
        MOBILE_PROTOCOL_VERSION, MOBILE_PROTOCOL_VERSION_V2,
    };
    use wisecrow_learning::mastery::{fold_accuracy, Attempt, CanonicalRating};
    use wisecrow_mobile::application::{
        CorpusRepository, GrammarRepository, LocalStore, MobileApi, MobileError, ProfileRepository,
        QueuedAttempt,
    };
    use wisecrow_mobile::storage::models::{Profile, ProfileIdentity};
    use wisecrow_mobile::storage::SqliteStore;
    use wisecrow_mobile::sync::{SyncEngine, SyncOutcome, SyncReason};

    /// A server that records what it is told, so the device's state can be
    /// compared against it rather than against a script.
    #[derive(Default)]
    struct RecordingServer {
        reachable: Mutex<bool>,
        received: Mutex<Vec<OfflineAttemptDto>>,
    }

    impl RecordingServer {
        fn go_online(&self) {
            *self.reachable.lock().expect("reachable") = true;
        }

        fn is_online(&self) -> Result<(), MobileError> {
            if *self.reachable.lock().expect("reachable") {
                Ok(())
            } else {
                Err(MobileError::Retryable)
            }
        }

        fn received(&self) -> Vec<OfflineAttemptDto> {
            self.received.lock().expect("received").clone()
        }

        /// What the server's own projection makes of what it has been told.
        ///
        /// It runs the same reduction the real server runs, so the comparison
        /// at the end is between two projections of one stream rather than
        /// between the device and a hard-coded number.
        fn projected(&self) -> (usize, f64) {
            let received = self.received();
            let attempts: Vec<Attempt> = received
                .iter()
                .map(|attempt| Attempt {
                    event_id: attempt.event_id,
                    session_id: attempt.session_id,
                    rule_id: 10,
                    correct: attempt.answer.to_lowercase() == "estoy",
                    hint_shown: attempt.hint_shown,
                    occurred_at: attempt.occurred_at,
                })
                .collect();
            let ratings: Vec<CanonicalRating> =
                wisecrow_learning::mastery::reduce_to_ratings(&attempts);
            let accuracy = attempts
                .iter()
                .fold(None, |running, attempt| {
                    Some(fold_accuracy(running, attempt.correct))
                })
                .unwrap_or_default();
            (ratings.len(), accuracy)
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

    fn served_item() -> OfflineGrammarItemDto {
        OfflineGrammarItemDto {
            item_id: 1,
            revision: 1,
            rule_id: 10,
            rule_slug: String::from("ser-vs-estar-permanent-qualities"),
            rule_title: String::from("Ser vs estar"),
            rule_explanation: String::from("Permanent against temporary."),
            level: String::from("A2"),
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
    impl MobileApi for RecordingServer {
        async fn capabilities(&self) -> Result<MobileCapabilitiesDto, MobileError> {
            self.is_online()?;
            Ok(v1_capabilities())
        }

        async fn capabilities_v2(&self) -> Result<Option<MobileCapabilitiesDto>, MobileError> {
            self.is_online()?;
            Ok(Some(MobileCapabilitiesDto {
                protocol_version: MOBILE_PROTOCOL_VERSION_V2,
                supported_features: vec![
                    MobileFeatureDto::GrammarBankSync,
                    MobileFeatureDto::GrammarMasterySync,
                    MobileFeatureDto::GrammarAttemptUpload,
                ],
                ..v1_capabilities()
            }))
        }

        async fn me(&self) -> Result<UserDto, MobileError> {
            self.is_online()?;
            Ok(UserDto {
                id: 7,
                display_name: String::from("Test User"),
            })
        }

        async fn languages(&self) -> Result<Vec<LanguageInfo>, MobileError> {
            self.is_online()?;
            Ok(vec![LanguageInfo {
                code: String::from("es"),
                name: String::from("Spanish"),
                script_direction: ScriptDirection::Ltr,
            }])
        }

        async fn upload_reviews(
            &self,
            _request: &ReviewBatchRequestDto,
        ) -> Result<ReviewBatchResponseDto, MobileError> {
            self.is_online()?;
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
            self.is_online()?;
            Ok(NbackBatchResponseDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                acknowledgements: Vec::new(),
            })
        }

        async fn card_changes(
            &self,
            request: &CardChangeRequestDto,
        ) -> Result<CardChangePageDto, MobileError> {
            self.is_online()?;
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
            self.is_online()?;
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
            self.is_online()?;
            Ok(GrammarBankChangePageDto {
                protocol_version: MOBILE_PROTOCOL_VERSION_V2,
                language: String::from("es"),
                changes: if request.cursor == 0 {
                    vec![GrammarBankChangeDto {
                        sequence: 1,
                        item_id: 1,
                        operation: GrammarChangeOperationDto::Upsert,
                        item: Some(served_item()),
                    }]
                } else {
                    Vec::new()
                },
                next_cursor: 1,
                has_more: false,
            })
        }

        async fn grammar_mastery_changes(
            &self,
            request: &GrammarMasteryChangeRequestDto,
        ) -> Result<GrammarMasteryChangePageDto, MobileError> {
            self.is_online()?;
            let (reps, accuracy) = self.projected();
            let changes = if request.cursor == 0 && reps > 0 {
                let now = Utc::now();
                vec![GrammarMasteryChangeDto {
                    sequence: 2,
                    rule_id: 10,
                    operation: GrammarChangeOperationDto::Upsert,
                    mastery: Some(GrammarMasteryStateDto {
                        rule_id: 10,
                        rule_slug: String::from("ser-vs-estar-permanent-qualities"),
                        stability: 3.0,
                        difficulty: 5.0,
                        elapsed_days: 0,
                        scheduled_days: 1,
                        reps: i32::try_from(reps).unwrap_or(i32::MAX),
                        lapses: 0,
                        state: 1,
                        accuracy: Some(accuracy as f32),
                        attempts: i32::try_from(self.received().len()).unwrap_or(i32::MAX),
                        last_review: Some(now),
                        due: now,
                    }),
                }]
            } else {
                Vec::new()
            };
            // An empty page leaves the cursor where it was, as the real feed
            // does: advancing past nothing would skip the change that has not
            // been written yet.
            let next_cursor = if changes.is_empty() {
                request.cursor
            } else {
                2
            };
            Ok(GrammarMasteryChangePageDto {
                protocol_version: MOBILE_PROTOCOL_VERSION_V2,
                changes,
                next_cursor,
                has_more: false,
            })
        }

        async fn upload_grammar_attempts(
            &self,
            request: &GrammarAttemptBatchRequestDto,
        ) -> Result<GrammarAttemptBatchResponseDto, MobileError> {
            self.is_online()?;
            let mut held = self.received.lock().expect("received");
            let mut results = Vec::with_capacity(request.attempts.len());
            for attempt in &request.attempts {
                let correct = attempt.answer.to_lowercase() == "estoy";
                let verdict = VerdictDto {
                    event_id: attempt.event_id,
                    correct,
                };
                if held.iter().any(|seen| seen.event_id == attempt.event_id) {
                    results.push(OfflineAttemptStatusDto::Duplicate(verdict));
                } else {
                    held.push(attempt.clone());
                    results.push(OfflineAttemptStatusDto::Accepted(verdict));
                }
            }
            Ok(GrammarAttemptBatchResponseDto {
                protocol_version: MOBILE_PROTOCOL_VERSION_V2,
                results,
            })
        }

        async fn login(
            &self,
            _email: &str,
            _password: &str,
        ) -> Result<MobileSessionDto, MobileError> {
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
                id: Uuid::from_u128(4242),
                origin: String::from("https://round-trip.example.test/"),
                imported_ca_fingerprint: None,
                active: true,
                created_at: now,
                updated_at: now,
            },
            user: UserDto {
                id: 7,
                display_name: String::from("Test User"),
            },
            device_id: Uuid::from_u128(99),
        }
    }

    fn pair() -> LanguagePairDto {
        LanguagePairDto {
            native_lang: String::from("en"),
            foreign_lang: String::from("es"),
        }
    }

    /// Reopening the store is what an app restart amounts to: the process is
    /// gone, the database file is not.
    async fn open(path: &std::path::Path) -> Arc<SqliteStore> {
        Arc::new(SqliteStore::open(path).await.expect("store"))
    }

    async fn seed(store: &SqliteStore) {
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
    }

    async fn sync(store: Arc<SqliteStore>, api: Arc<RecordingServer>) -> SyncOutcome {
        let store_resource: Arc<dyn LocalStore> = store;
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
    async fn offline_grammar_answer_survives_restart_and_converges() {
        let directory = tempdir().expect("temporary directory");
        let path = directory.path().join("round-trip.sqlite3");
        let api = Arc::new(RecordingServer::default());
        api.go_online();

        // The bank arrives while there is a network.
        let store = open(&path).await;
        seed(&store).await;
        let outcome = sync(Arc::clone(&store), Arc::clone(&api)).await;
        assert!(
            matches!(outcome, SyncOutcome::Complete { .. }),
            "{outcome:?}"
        );
        let held = store.grammar_items("es").await.expect("items");
        assert_eq!(held.len(), 1, "the device holds the bank");

        // The network goes away and the learner answers anyway.
        *api.reachable.lock().expect("reachable") = false;
        let session_id = Uuid::from_u128(7);
        let answer = QueuedAttempt {
            event_id: Uuid::from_u128(8),
            session_id,
            item_id: held[0].item_id,
            revision: held[0].revision,
            answer: String::from("estoy"),
            chose_option: false,
            hint_shown: false,
            ordinal: 1,
            occurred_at: Utc::now(),
            language: String::from("es"),
            level: Some(String::from("A2")),
        };
        store.queue_attempt(&answer).await.expect("queue");
        assert_eq!(store.pending_attempts(10).await.expect("pending").len(), 1);

        let projected_offline = store.grammar_mastery("es").await.expect("mastery");
        assert_eq!(
            projected_offline[0].accuracy,
            Some(1.0),
            "the map colours before the server has heard anything"
        );

        // A sync with no network must change nothing but the error it records.
        let outcome = sync(Arc::clone(&store), Arc::clone(&api)).await;
        assert!(matches!(outcome, SyncOutcome::Retryable), "{outcome:?}");
        assert_eq!(store.pending_attempts(10).await.expect("pending").len(), 1);

        // The app is killed.
        drop(store);
        let store = open(&path).await;
        assert_eq!(
            store.pending_attempts(10).await.expect("pending").len(),
            1,
            "the outbox survives a restart"
        );

        // The network returns.
        api.go_online();
        let outcome = sync(Arc::clone(&store), Arc::clone(&api)).await;
        assert!(
            matches!(outcome, SyncOutcome::Complete { .. }),
            "{outcome:?}"
        );
        assert!(
            store
                .pending_attempts(10)
                .await
                .expect("pending")
                .is_empty(),
            "the outbox drains"
        );

        let received = api.received();
        assert_eq!(received.len(), 1, "the answer arrived exactly once");
        assert_eq!(received[0].event_id, answer.event_id);
        assert_eq!(received[0].session_id, session_id);
        assert_eq!(received[0].revision, held[0].revision);

        let (reps, accuracy) = api.projected();
        assert_eq!(reps, 1, "the server converges on the device's state");

        let mirrored = store.grammar_mastery("es").await.expect("mastery");
        assert_eq!(mirrored.len(), 1);
        assert_eq!(mirrored[0].reps, 1);
        assert!(
            (f64::from(mirrored[0].accuracy.unwrap_or_default()) - accuracy).abs() < 1e-6,
            "device and server agree on the figure that colours the map"
        );

        // Syncing again must not resend what the server already holds.
        let outcome = sync(Arc::clone(&store), Arc::clone(&api)).await;
        assert!(
            matches!(outcome, SyncOutcome::Complete { .. }),
            "{outcome:?}"
        );
        assert_eq!(api.received().len(), 1, "nothing is counted twice");
    }
}
