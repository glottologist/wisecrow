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
