use std::collections::BTreeMap;

use chrono::Utc;
use proptest::prelude::*;
use tempfile::tempdir;
use uuid::Uuid;
use wisecrow_dto::{
    CardChangeDto, CardChangePageDto, CardSnapshotDto, CardStatusDto, CorpusChangeDto,
    CorpusChangeKindDto, CorpusChangePageDto, CorpusPageDto, CorpusTranslationDto, LanguagePairDto,
    UserDto, MOBILE_PROTOCOL_VERSION,
};
use wisecrow_mobile::application::{CorpusRepository, ProfileRepository};
use wisecrow_mobile::storage::models::{PairStatus, Profile, ProfileIdentity};
use wisecrow_mobile::storage::SqliteStore;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn snapshot_reapplication_is_idempotent(
        entries in prop::collection::btree_map(
            1i32..1_000,
            ("[a-z]{1,12}", "[a-z]{1,12}", 0i32..1_000, any::<bool>()),
            1..8,
        ),
        reapplications in 1usize..6,
    ) {
        let runtime = tokio::runtime::Runtime::new().expect("runtime");
        runtime.block_on(assert_snapshot_reapplication(entries, reapplications));
    }
}

async fn assert_snapshot_reapplication(
    entries: BTreeMap<i32, (String, String, i32, bool)>,
    reapplications: usize,
) {
    let directory = tempdir().expect("temporary directory");
    let store = SqliteStore::open(&directory.path().join("corpus.sqlite3"))
        .await
        .expect("store");
    save_identity(&store, test_identity("https://one.example.test/"))
        .await
        .expect("identity");
    let pair = test_pair();
    store
        .begin_snapshot(&pair, 42, Some(4_096))
        .await
        .expect("begin snapshot");
    let page = snapshot_page(&pair, &entries, false);

    for _ in 0..reapplications {
        store
            .apply_snapshot_page(&page)
            .await
            .expect("apply snapshot");
    }
    assert_reapplied_snapshot(&store, &pair, &entries, page.next_cursor).await;
}

async fn assert_reapplied_snapshot(
    store: &SqliteStore,
    pair: &LanguagePairDto,
    entries: &BTreeMap<i32, (String, String, i32, bool)>,
    expected_cursor: i32,
) {
    assert_eq!(
        store.pair_status(pair).await.expect("status"),
        PairStatus::Ready
    );
    let estimate = store.corpus_estimate(pair).await.expect("estimate");
    assert_eq!(
        estimate.translation_count,
        u64::try_from(entries.len()).expect("count")
    );
    let expected_bytes = entries
        .values()
        .map(|(from, to, _, _)| from.len().saturating_add(to.len()))
        .map(|bytes| u64::try_from(bytes).expect("text bytes"))
        .sum::<u64>();
    assert_eq!(estimate.text_bytes, expected_bytes);
    for (translation_id, (from, to, frequency, is_phrase)) in entries {
        let stored = store
            .translation(pair, *translation_id)
            .await
            .expect("lookup")
            .expect("stored translation");
        assert_eq!(&stored.from_phrase, from);
        assert_eq!(&stored.to_phrase, to);
        assert_eq!(stored.frequency, *frequency);
        assert_eq!(stored.is_phrase, *is_phrase);
    }
    let cursor: i32 = sqlx::query_scalar("SELECT snapshot_after_id FROM language_pairs")
        .fetch_one(store.pool())
        .await
        .expect("snapshot cursor");
    assert_eq!(cursor, expected_cursor);
}

#[tokio::test]
async fn snapshot_changes_are_atomic_scoped_and_ranked() {
    let directory = tempdir().expect("temporary directory");
    let store = SqliteStore::open(&directory.path().join("changes.sqlite3"))
        .await
        .expect("store");
    let first_identity = test_identity("https://one.example.test/");
    save_identity(&store, first_identity.clone())
        .await
        .expect("first identity");
    let pair = test_pair();
    seed_initial_snapshot(&store, &pair).await;
    assert_mismatched_pair_is_rejected(&store).await;
    assert_interrupted_change_rolls_back(&store, &pair).await;
    let second_identity = seed_other_profile(&store, &pair).await;
    store
        .activate_profile(first_identity.profile.id)
        .await
        .expect("activate first");
    apply_changes_and_assert_scope(&store, &pair, second_identity.profile.id).await;
}

async fn assert_mismatched_pair_is_rejected(store: &SqliteStore) {
    let pair = LanguagePairDto {
        native_lang: String::from("en"),
        foreign_lang: String::from("fr"),
    };
    let page = CorpusChangePageDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        pair,
        changes: Vec::new(),
        next_cursor: 0,
        has_more: false,
        change_watermark: 0,
    };
    assert!(store.apply_change_page(&page).await.is_err());
}

async fn seed_initial_snapshot(store: &SqliteStore, pair: &LanguagePairDto) {
    store
        .begin_snapshot(pair, 10, Some(1_024))
        .await
        .expect("begin snapshot");
    store
        .apply_snapshot_page(&fixed_snapshot(pair, 1, true))
        .await
        .expect("first page");
    assert_eq!(
        store.pair_status(pair).await.expect("status"),
        PairStatus::Downloading
    );
    store
        .apply_snapshot_page(&fixed_snapshot(pair, 2, false))
        .await
        .expect("final page");
    assert_eq!(
        store.pair_status(pair).await.expect("status"),
        PairStatus::Ready
    );
}

async fn assert_interrupted_change_rolls_back(store: &SqliteStore, pair: &LanguagePairDto) {
    let interrupted = CorpusChangePageDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        pair: pair.clone(),
        changes: vec![
            upsert_change(11, translation(1, "changed", "uno", 99)),
            invalid_change(12),
        ],
        next_cursor: 12,
        has_more: false,
        change_watermark: 12,
    };
    assert!(store.apply_change_page(&interrupted).await.is_err());
    assert_eq!(
        store
            .translation(pair, 1)
            .await
            .expect("lookup")
            .expect("translation")
            .from_phrase,
        "from-1"
    );
    assert_eq!(change_cursor(store, pair).await, 10);
}

async fn seed_other_profile(store: &SqliteStore, pair: &LanguagePairDto) -> ProfileIdentity {
    let second_identity = test_identity("https://two.example.test/");
    save_identity(store, second_identity.clone())
        .await
        .expect("second identity");
    store
        .begin_snapshot(pair, 20, None)
        .await
        .expect("second snapshot");
    let mut second_page = fixed_snapshot(pair, 2, false);
    second_page.snapshot_watermark = 20;
    store
        .apply_snapshot_page(&second_page)
        .await
        .expect("second profile page");
    second_identity
}

async fn apply_changes_and_assert_scope(
    store: &SqliteStore,
    pair: &LanguagePairDto,
    other_profile_id: Uuid,
) {
    let changes = CorpusChangePageDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        pair: pair.clone(),
        changes: vec![
            upsert_change(11, translation(1, "changed", "uno", 99)),
            delete_change(12, 2),
        ],
        next_cursor: 12,
        has_more: false,
        change_watermark: 12,
    };
    store.apply_change_page(&changes).await.expect("changes");
    assert!(store.translation(pair, 2).await.expect("lookup").is_none());
    assert_eq!(
        other_profile_translation_count(store, other_profile_id).await,
        1
    );
    let ranked = store
        .ranked_translations(pair, 10)
        .await
        .expect("ranked translations");
    assert_eq!(ranked.first().map(|item| item.translation_id), Some(1));
    let regressed = CorpusChangePageDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        pair: pair.clone(),
        changes: Vec::new(),
        next_cursor: 11,
        has_more: false,
        change_watermark: 11,
    };
    assert!(store.apply_change_page(&regressed).await.is_err());
    assert_eq!(change_cursor(store, pair).await, 12);
}

#[tokio::test]
async fn card_tombstone_waits_for_pending_review_and_preserves_other_profile() {
    let directory = tempdir().expect("temporary directory");
    let store = SqliteStore::open(&directory.path().join("cards.sqlite3"))
        .await
        .expect("store");
    let first = test_identity("https://one.example.test/");
    save_identity(&store, first.clone())
        .await
        .expect("first identity");
    store
        .apply_card_page(&card_upsert_page(1))
        .await
        .expect("first card");
    insert_pending_review(&store, &first).await;

    let tombstone = card_delete_page(2);
    assert!(store.apply_card_page(&tombstone).await.is_err());
    assert_eq!(card_count(&store, first.profile.id).await, 1);
    assert_eq!(card_cursor(&store, first.profile.id).await, 1);

    let second = test_identity("https://two.example.test/");
    save_identity(&store, second.clone())
        .await
        .expect("second identity");
    store
        .apply_card_page(&card_upsert_page(1))
        .await
        .expect("second card");
    store
        .activate_profile(first.profile.id)
        .await
        .expect("activate first");
    sqlx::query("DELETE FROM review_outbox WHERE profile_id = ? AND user_id = ?")
        .bind(first.profile.id)
        .bind(first.user.id)
        .execute(store.pool())
        .await
        .expect("remove pending review");
    store
        .apply_card_page(&tombstone)
        .await
        .expect("card delete");

    assert_eq!(card_count(&store, first.profile.id).await, 0);
    assert_eq!(card_count(&store, second.profile.id).await, 1);
    assert_eq!(card_cursor(&store, first.profile.id).await, 2);
}

#[tokio::test]
async fn empty_snapshot_becomes_ready_at_its_watermark() {
    let directory = tempdir().expect("temporary directory");
    let store = SqliteStore::open(&directory.path().join("empty.sqlite3"))
        .await
        .expect("store");
    save_identity(&store, test_identity("https://empty.example.test/"))
        .await
        .expect("identity");
    let pair = test_pair();
    store
        .begin_snapshot(&pair, 5, Some(0))
        .await
        .expect("begin snapshot");
    let page = CorpusPageDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        pair: pair.clone(),
        translations: Vec::new(),
        next_cursor: 0,
        has_more: false,
        snapshot_watermark: 5,
    };

    store.apply_snapshot_page(&page).await.expect("empty page");

    assert_eq!(
        store.pair_status(&pair).await.expect("status"),
        PairStatus::Ready
    );
    assert_eq!(change_cursor(&store, &pair).await, 5);
}

fn snapshot_page(
    pair: &LanguagePairDto,
    entries: &BTreeMap<i32, (String, String, i32, bool)>,
    has_more: bool,
) -> CorpusPageDto {
    let translations = entries
        .iter()
        .map(
            |(id, (from, to, frequency, is_phrase))| CorpusTranslationDto {
                translation_id: *id,
                from_phrase: from.clone(),
                to_phrase: to.clone(),
                frequency: *frequency,
                is_phrase: *is_phrase,
            },
        )
        .collect::<Vec<_>>();
    CorpusPageDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        pair: pair.clone(),
        next_cursor: entries.last_key_value().map_or(0, |(id, _)| *id),
        translations,
        has_more,
        snapshot_watermark: 42,
    }
}

fn fixed_snapshot(pair: &LanguagePairDto, translation_id: i32, has_more: bool) -> CorpusPageDto {
    let mut entries = BTreeMap::new();
    entries.insert(
        translation_id,
        (
            String::from("from-") + &translation_id.to_string(),
            String::from("to-") + &translation_id.to_string(),
            translation_id.saturating_mul(10),
            false,
        ),
    );
    let mut page = snapshot_page(pair, &entries, has_more);
    page.snapshot_watermark = 10;
    page
}

fn translation(id: i32, from: &str, to: &str, frequency: i32) -> CorpusTranslationDto {
    CorpusTranslationDto {
        translation_id: id,
        from_phrase: String::from(from),
        to_phrase: String::from(to),
        frequency,
        is_phrase: false,
    }
}

fn upsert_change(sequence: i64, item: CorpusTranslationDto) -> CorpusChangeDto {
    CorpusChangeDto {
        sequence,
        translation_id: item.translation_id,
        kind: CorpusChangeKindDto::Upsert,
        translation: Some(item),
        changed_at: Utc::now(),
    }
}

fn invalid_change(sequence: i64) -> CorpusChangeDto {
    CorpusChangeDto {
        sequence,
        translation_id: 2,
        kind: CorpusChangeKindDto::Upsert,
        translation: None,
        changed_at: Utc::now(),
    }
}

fn delete_change(sequence: i64, translation_id: i32) -> CorpusChangeDto {
    CorpusChangeDto {
        sequence,
        translation_id,
        kind: CorpusChangeKindDto::Delete,
        translation: None,
        changed_at: Utc::now(),
    }
}

fn card_upsert_page(sequence: i64) -> CardChangePageDto {
    CardChangePageDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        changes: vec![CardChangeDto::Upsert {
            sequence,
            card: test_card(sequence),
        }],
        next_cursor: sequence,
        has_more: false,
        change_watermark: sequence,
    }
}

fn card_delete_page(sequence: i64) -> CardChangePageDto {
    CardChangePageDto {
        protocol_version: MOBILE_PROTOCOL_VERSION,
        changes: vec![CardChangeDto::Delete {
            sequence,
            translation_id: 1,
        }],
        next_cursor: sequence,
        has_more: false,
        change_watermark: sequence,
    }
}

fn test_card(server_cursor: i64) -> CardSnapshotDto {
    CardSnapshotDto {
        translation_id: 1,
        stability: 1.0,
        difficulty: 5.0,
        elapsed_days: 0,
        scheduled_days: 1,
        reps: 1,
        lapses: 0,
        state: CardStatusDto::Learning,
        last_review: None,
        due: Utc::now(),
        server_cursor,
    }
}

async fn insert_pending_review(store: &SqliteStore, identity: &ProfileIdentity) {
    sqlx::query(
        "INSERT INTO review_outbox
         (profile_id, user_id, event_id, device_id, translation_id, rating, occurred_at, status)
         VALUES (?, ?, ?, ?, 1, 3, ?, 'pending')",
    )
    .bind(identity.profile.id)
    .bind(identity.user.id)
    .bind(Uuid::new_v4())
    .bind(identity.device_id)
    .bind(Utc::now())
    .execute(store.pool())
    .await
    .expect("pending review");
}

async fn save_identity(
    store: &SqliteStore,
    identity: ProfileIdentity,
) -> Result<(), wisecrow_mobile::application::MobileError> {
    store.save_profile_identity(&identity).await
}

fn test_identity(origin: &str) -> ProfileIdentity {
    let now = Utc::now();
    ProfileIdentity {
        profile: Profile {
            id: Uuid::new_v4(),
            origin: String::from(origin),
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

fn test_pair() -> LanguagePairDto {
    LanguagePairDto {
        native_lang: String::from("en"),
        foreign_lang: String::from("es"),
    }
}

async fn change_cursor(store: &SqliteStore, pair: &LanguagePairDto) -> i64 {
    sqlx::query_scalar(
        "SELECT change_cursor FROM language_pairs WHERE native = ? AND \"foreign\" = ?",
    )
    .bind(&pair.native_lang)
    .bind(&pair.foreign_lang)
    .fetch_one(store.pool())
    .await
    .expect("change cursor")
}

async fn other_profile_translation_count(store: &SqliteStore, profile_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM translations WHERE profile_id = ?")
        .bind(profile_id)
        .fetch_one(store.pool())
        .await
        .expect("other profile count")
}

async fn card_count(store: &SqliteStore, profile_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM cards WHERE profile_id = ?")
        .bind(profile_id)
        .fetch_one(store.pool())
        .await
        .expect("card count")
}

async fn card_cursor(store: &SqliteStore, profile_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT card_cursor FROM sync_state WHERE profile_id = ?")
        .bind(profile_id)
        .fetch_one(store.pool())
        .await
        .expect("card cursor")
}
