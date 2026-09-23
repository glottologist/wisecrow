//! The local grammar store: the mirrored bank, and the outbox that owes the
//! server.
//!
//! The properties worth holding are the ones an offline session depends on:
//! that an item arrives complete enough to be posed and marked without a
//! network, that a queued answer survives the app being killed, and that the
//! outbox hands answers back in the order they were taken.

use chrono::{Duration, Utc};
use tempfile::tempdir;
use uuid::Uuid;
use wisecrow_dto::{
    GrammarAttemptBatchResponseDto, GrammarBankChangeDto, GrammarBankChangePageDto,
    GrammarChangeOperationDto, GrammarMasteryChangeDto, GrammarMasteryChangePageDto,
    GrammarMasteryStateDto, GrammarOptionDto, OfflineAttemptStatusDto, OfflineGrammarItemDto,
    UserDto, VerdictDto, MOBILE_PROTOCOL_VERSION_V2,
};
use wisecrow_mobile::application::{
    GrammarRepository, MobileError, ProfileRepository, QueuedAttempt,
};
use wisecrow_mobile::storage::models::{Profile, ProfileIdentity};
use wisecrow_mobile::storage::SqliteStore;

type TestResult = Result<(), MobileError>;

fn identity() -> ProfileIdentity {
    let now = Utc::now();
    ProfileIdentity {
        profile: Profile {
            id: Uuid::new_v4(),
            origin: String::from("https://grammar.example.test/"),
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

fn item(item_id: i32, rule_id: i32) -> OfflineGrammarItemDto {
    OfflineGrammarItemDto {
        item_id,
        revision: 1,
        rule_id,
        rule_slug: format!("rule-{rule_id}"),
        rule_title: String::from("Ser vs estar"),
        rule_explanation: String::from("Permanent against temporary."),
        level: String::from("A1"),
        language: String::from("es"),
        prompt: String::from("Yo ___ cansado."),
        hint: Some(String::from("A passing state.")),
        options: vec![GrammarOptionDto {
            id: String::from("o1"),
            text: String::from("estoy"),
        }],
        answer: Some(String::from("estoy")),
        accepted: vec![String::from("estoy mal")],
        correct_option: Some(String::from("o1")),
    }
}

fn bank_page(changes: Vec<GrammarBankChangeDto>, next_cursor: i64) -> GrammarBankChangePageDto {
    GrammarBankChangePageDto {
        protocol_version: MOBILE_PROTOCOL_VERSION_V2,
        language: String::from("es"),
        changes,
        next_cursor,
        has_more: false,
    }
}

fn queued(ordinal: u32, seconds: i64, session: Uuid) -> QueuedAttempt {
    QueuedAttempt {
        event_id: Uuid::new_v4(),
        session_id: session,
        item_id: 1,
        revision: 1,
        answer: String::from("estoy"),
        chose_option: false,
        hint_shown: false,
        ordinal,
        occurred_at: Utc::now() + Duration::seconds(seconds),
        language: String::from("es"),
        level: Some(String::from("A1")),
    }
}

async fn store(path: &std::path::Path) -> SqliteStore {
    SqliteStore::open(path).await.expect("store")
}

/// An item must arrive complete: a device that cannot mark an answer offline
/// cannot run a session offline.
#[tokio::test]
async fn a_mirrored_item_carries_everything_needed_to_mark_it() -> TestResult {
    let directory = tempdir().expect("temporary directory");
    let store = store(&directory.path().join("grammar.sqlite3")).await;
    store.save_profile_identity(&identity()).await?;

    store
        .apply_bank_page(&bank_page(
            vec![GrammarBankChangeDto {
                sequence: 4,
                item_id: 1,
                operation: GrammarChangeOperationDto::Upsert,
                item: Some(item(1, 10)),
            }],
            4,
        ))
        .await?;

    let held = store.grammar_items("es").await?;
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].answer.as_deref(), Some("estoy"));
    assert_eq!(held[0].accepted, vec![String::from("estoy mal")]);
    assert_eq!(held[0].options[0].id, "o1");
    assert_eq!(held[0].correct_option.as_deref(), Some("o1"));

    let rules = store.grammar_rules("es").await?;
    assert_eq!(rules.len(), 1, "the point behind the item comes with it");
    assert_eq!(rules[0].level, "A1");

    assert_eq!(store.grammar_cursors("es").await?.bank, 4);
    Ok(())
}

/// An item the server withholds is one the device must stop holding, whether
/// it was deleted or merely taken out of service.
#[tokio::test]
async fn an_item_the_server_withholds_is_dropped() -> TestResult {
    let directory = tempdir().expect("temporary directory");
    let store = store(&directory.path().join("grammar.sqlite3")).await;
    store.save_profile_identity(&identity()).await?;

    store
        .apply_bank_page(&bank_page(
            vec![GrammarBankChangeDto {
                sequence: 1,
                item_id: 1,
                operation: GrammarChangeOperationDto::Upsert,
                item: Some(item(1, 10)),
            }],
            1,
        ))
        .await?;
    assert_eq!(store.grammar_items("es").await?.len(), 1);

    store
        .apply_bank_page(&bank_page(
            vec![GrammarBankChangeDto {
                sequence: 2,
                item_id: 1,
                operation: GrammarChangeOperationDto::Upsert,
                item: None,
            }],
            2,
        ))
        .await?;
    assert!(
        store.grammar_items("es").await?.is_empty(),
        "an unpromoted item is dropped as surely as a deleted one"
    );
    Ok(())
}

/// Killing the app is the ordinary case, not the exceptional one.
#[tokio::test]
async fn a_queued_answer_survives_the_store_being_reopened() -> TestResult {
    let directory = tempdir().expect("temporary directory");
    let path = directory.path().join("grammar.sqlite3");
    let session = Uuid::new_v4();

    let attempt = {
        let store = store(&path).await;
        store.save_profile_identity(&identity()).await?;
        let attempt = queued(1, 0, session);
        store.queue_attempt(&attempt).await?;
        assert_eq!(store.pending_attempts(10).await?.len(), 1);
        attempt
    };

    let reopened = store(&path).await;
    let pending = reopened.pending_attempts(10).await?;
    assert_eq!(pending.len(), 1, "the outbox is not a cache");
    assert_eq!(pending[0].event_id, attempt.event_id);
    assert_eq!(pending[0].ordinal, 1);
    assert_eq!(pending[0].level.as_deref(), Some("A1"));
    Ok(())
}

/// The server's rating rule depends on which answer came first within a
/// session, so the order the device took them in has to survive the journey.
#[tokio::test]
async fn the_outbox_drains_in_the_order_answers_were_taken() -> TestResult {
    let directory = tempdir().expect("temporary directory");
    let store = store(&directory.path().join("grammar.sqlite3")).await;
    store.save_profile_identity(&identity()).await?;
    let session = Uuid::new_v4();

    let first = queued(1, 0, session);
    let second = queued(2, 5, session);
    let third = queued(3, 10, session);
    for attempt in [&first, &second, &third] {
        store.queue_attempt(attempt).await?;
    }
    // Queuing one already held must not move it to the back of the queue.
    store.queue_attempt(&first).await?;

    let pending = store.pending_attempts(10).await?;
    assert_eq!(pending.len(), 3, "a repeat queues nothing new");
    assert_eq!(
        pending.iter().map(|a| a.ordinal).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );

    store
        .apply_attempt_response(&GrammarAttemptBatchResponseDto {
            protocol_version: MOBILE_PROTOCOL_VERSION_V2,
            results: vec![
                OfflineAttemptStatusDto::Accepted(VerdictDto {
                    event_id: first.event_id,
                    correct: true,
                }),
                OfflineAttemptStatusDto::Duplicate(VerdictDto {
                    event_id: second.event_id,
                    correct: false,
                }),
            ],
        })
        .await?;

    let left = store.pending_attempts(10).await?;
    assert_eq!(left.len(), 1, "an acknowledged answer leaves the outbox");
    assert_eq!(left[0].event_id, third.event_id);
    Ok(())
}

/// An answer taken offline must colour the map at once, or the learner sees
/// nothing happen until the next time the device reaches the server.
#[tokio::test]
async fn an_answer_taken_offline_colours_its_point_before_the_server_hears_of_it() -> TestResult {
    let directory = tempdir().expect("temporary directory");
    let store = store(&directory.path().join("grammar.sqlite3")).await;
    store.save_profile_identity(&identity()).await?;
    // A cloze, so that grading exercises the text path a typed answer takes.
    let cloze = OfflineGrammarItemDto {
        item_id: 2,
        options: Vec::new(),
        correct_option: None,
        ..item(2, 10)
    };
    store
        .apply_bank_page(&bank_page(
            vec![GrammarBankChangeDto {
                sequence: 1,
                item_id: 2,
                operation: GrammarChangeOperationDto::Upsert,
                item: Some(cloze),
            }],
            1,
        ))
        .await?;

    let session = Uuid::new_v4();
    let mut wrong = queued(1, 0, session);
    wrong.item_id = 2;
    wrong.answer = String::from("soy");
    store.queue_attempt(&wrong).await?;

    let after_wrong = store.grammar_mastery("es").await?;
    assert_eq!(
        after_wrong.len(),
        1,
        "a point the learner met is no longer unseen"
    );
    assert_eq!(after_wrong[0].attempts, 1);
    assert_eq!(after_wrong[0].accuracy, Some(0.0));

    // The device marks with the same grader the server uses, accents and case
    // folded alike.
    let mut right = queued(1, 5, Uuid::new_v4());
    right.item_id = 2;
    right.answer = String::from("ESTOY");
    store.queue_attempt(&right).await?;

    let after_right = store.grammar_mastery("es").await?;
    assert_eq!(after_right[0].attempts, 2);
    assert!(
        after_right[0].accuracy.unwrap_or_default() > 0.0,
        "a correct answer moves the figure it colours"
    );

    // Queuing the same answer twice must not count it twice.
    store.queue_attempt(&right).await?;
    assert_eq!(store.grammar_mastery("es").await?[0].attempts, 2);
    Ok(())
}

/// The mastery mirror is what the brainmap is drawn from offline.
#[tokio::test]
async fn mastery_is_mirrored_for_the_points_the_device_holds() -> TestResult {
    let directory = tempdir().expect("temporary directory");
    let store = store(&directory.path().join("grammar.sqlite3")).await;
    store.save_profile_identity(&identity()).await?;

    store
        .apply_bank_page(&bank_page(
            vec![GrammarBankChangeDto {
                sequence: 1,
                item_id: 1,
                operation: GrammarChangeOperationDto::Upsert,
                item: Some(item(1, 10)),
            }],
            1,
        ))
        .await?;

    let due = Utc::now();
    store
        .apply_mastery_page(&GrammarMasteryChangePageDto {
            protocol_version: MOBILE_PROTOCOL_VERSION_V2,
            changes: vec![GrammarMasteryChangeDto {
                sequence: 3,
                rule_id: 10,
                operation: GrammarChangeOperationDto::Upsert,
                mastery: Some(GrammarMasteryStateDto {
                    rule_id: 10,
                    rule_slug: String::from("rule-10"),
                    stability: 2.5,
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
            }],
            next_cursor: 3,
            has_more: false,
        })
        .await?;

    let mastery = store.grammar_mastery("es").await?;
    assert_eq!(mastery.len(), 1);
    assert_eq!(mastery[0].reps, 1);
    assert_eq!(mastery[0].accuracy, Some(1.0));
    assert_eq!(store.grammar_cursors("es").await?.mastery, 3);
    Ok(())
}
