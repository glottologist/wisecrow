//! IDOR / ownership regression tests: the per-user repository mutators must fail
//! closed when the caller does not own the target session.

use sqlx::PgPool;

use wisecrow::dnb::scoring::{apply_adaptation, channel_accuracy, AdaptationState, Channel};
use wisecrow::dnb::session::DnbSessionRepository;
use wisecrow::dnb::{CompletedTrial, DnbVocab, Trial, TrialResponse};
use wisecrow::errors::WisecrowError;
use wisecrow::srs::session::SessionManager;

async fn test_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".to_owned());
    let pool = PgPool::connect(&url)
        .await
        .expect("Failed to connect to test database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");
    pool
}

async fn make_user(pool: &PgPool, email: &str) -> i32 {
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(email)
        .execute(pool)
        .await
        .expect("cleanup user");
    sqlx::query_scalar("INSERT INTO users (display_name, email) VALUES ('T', $1) RETURNING id")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("insert user")
}

async fn ensure_lang(pool: &PgPool, code: &str, name: &str) -> i32 {
    if let Some(id) = sqlx::query_scalar::<_, i32>("SELECT id FROM languages WHERE code = $1")
        .bind(code)
        .fetch_optional(pool)
        .await
        .expect("lang lookup")
    {
        return id;
    }
    sqlx::query_scalar("INSERT INTO languages (code, name) VALUES ($1, $2) RETURNING id")
        .bind(code)
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("insert lang")
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn srs_pause_complete_reject_other_user() {
    let pool = test_pool().await;
    let owner = make_user(&pool, "own-a@test.local").await;
    let attacker = make_user(&pool, "own-b@test.local").await;

    let sid: i32 = sqlx::query_scalar(
        "INSERT INTO sessions (user_id, native_lang, foreign_lang, deck_size, speed_ms)
         VALUES ($1, 'en', 'ru', 0, 3000) RETURNING id",
    )
    .bind(owner)
    .fetch_one(&pool)
    .await
    .expect("insert session");

    assert!(matches!(
        SessionManager::pause(&pool, sid, attacker).await,
        Err(WisecrowError::Unauthorized)
    ));
    assert!(matches!(
        SessionManager::complete(&pool, sid, attacker).await,
        Err(WisecrowError::Unauthorized)
    ));

    assert!(SessionManager::pause(&pool, sid, owner).await.is_ok());
    assert!(SessionManager::complete(&pool, sid, owner).await.is_ok());

    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![owner, attacker])
        .execute(&pool)
        .await
        .expect("cleanup");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn dnb_save_and_complete_reject_other_user() {
    let pool = test_pool().await;
    let owner = make_user(&pool, "own-c@test.local").await;
    let attacker = make_user(&pool, "own-d@test.local").await;

    let en = ensure_lang(&pool, "en", "English").await;
    let ru = ensure_lang(&pool, "ru", "Russian").await;
    let tid: i32 = sqlx::query_scalar(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase, frequency)
         VALUES ($1, $2, 'hello', 'privet', 1) RETURNING id",
    )
    .bind(en)
    .bind(ru)
    .fetch_one(&pool)
    .await
    .expect("insert translation");

    let sid: i32 = sqlx::query_scalar(
        "INSERT INTO dnb_sessions (user_id, native_lang, foreign_lang, mode, interval_ms_start, interval_ms_end)
         VALUES ($1, 'ru', 'en', 'audio_written', 3000, 3000) RETURNING id",
    )
    .bind(owner)
    .fetch_one(&pool)
    .await
    .expect("insert dnb session");

    let vocab = DnbVocab {
        translation_id: tid,
        from_phrase: "hello".to_owned(),
        to_phrase: "privet".to_owned(),
    };
    let trial = CompletedTrial {
        trial: Trial {
            trial_number: 1,
            n_level: 2,
            audio_vocab: vocab.clone(),
            visual_vocab: vocab,
            audio_match: false,
            visual_match: false,
            interval_ms: 3000,
        },
        response: TrialResponse {
            audio_response: Some(false),
            visual_response: Some(false),
            response_time_ms: Some(500),
        },
    };
    let state = AdaptationState::new(2, 3000);

    // EXISTS-guarded insert: attacker cannot write a trial to the owner's session.
    assert!(matches!(
        DnbSessionRepository::save_trial(&pool, sid, attacker, &trial).await,
        Err(WisecrowError::Unauthorized)
    ));
    assert!(matches!(
        DnbSessionRepository::complete_session(&pool, sid, attacker, &state, 0, None, None).await,
        Err(WisecrowError::Unauthorized)
    ));

    // Owner succeeds.
    assert!(DnbSessionRepository::save_trial(&pool, sid, owner, &trial)
        .await
        .is_ok());
    assert!(DnbSessionRepository::complete_session(
        &pool,
        sid,
        owner,
        &state,
        1,
        Some(0.0),
        Some(0.0)
    )
    .await
    .is_ok());

    // Confirm exactly one trial was written (the attacker's was rejected).
    let trial_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM dnb_trials WHERE session_id = $1")
            .bind(sid)
            .fetch_one(&pool)
            .await
            .expect("count trials");
    assert_eq!(trial_count, 1);

    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![owner, attacker])
        .execute(&pool)
        .await
        .expect("cleanup");
    sqlx::query("DELETE FROM translations WHERE id = $1")
        .bind(tid)
        .execute(&pool)
        .await
        .expect("cleanup translation");
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn dnb_trials_and_state_roundtrip() {
    let pool = test_pool().await;
    let owner = make_user(&pool, "dnb-rt@test.local").await;
    let other = make_user(&pool, "dnb-rt-other@test.local").await;

    let en = ensure_lang(&pool, "en", "English").await;
    let ru = ensure_lang(&pool, "ru", "Russian").await;
    let tid: i32 = sqlx::query_scalar(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase, frequency)
         VALUES ($1, $2, 'roundtrip', 'roundtrip', 1) RETURNING id",
    )
    .bind(en)
    .bind(ru)
    .fetch_one(&pool)
    .await
    .expect("insert translation");

    let sid: i32 = sqlx::query_scalar(
        "INSERT INTO dnb_sessions (user_id, native_lang, foreign_lang, mode, interval_ms_start, interval_ms_end)
         VALUES ($1, 'ru', 'en', 'audio_written', 4000, 4000) RETURNING id",
    )
    .bind(owner)
    .fetch_one(&pool)
    .await
    .expect("insert dnb session");

    // Five all-correct trials persist with a real translation id (no FK error).
    for i in 1..=5u32 {
        let vocab = DnbVocab {
            translation_id: tid,
            from_phrase: "roundtrip".to_owned(),
            to_phrase: "roundtrip".to_owned(),
        };
        let trial = CompletedTrial {
            trial: Trial {
                trial_number: i,
                n_level: 2,
                audio_vocab: vocab.clone(),
                visual_vocab: vocab,
                audio_match: true,
                visual_match: true,
                interval_ms: 4000,
            },
            response: TrialResponse {
                audio_response: Some(true),
                visual_response: Some(true),
                response_time_ms: Some(500),
            },
        };
        DnbSessionRepository::save_trial(&pool, sid, owner, &trial)
            .await
            .expect("save trial");
    }

    let trials = DnbSessionRepository::load_answered_trials(&pool, sid)
        .await
        .expect("load trials");
    assert_eq!(trials.len(), 5);
    assert!((channel_accuracy(&trials, Channel::Audio, 5) - 1.0).abs() < f64::EPSILON);

    // High accuracy over the window raises the n-level; the persisted state shows it.
    let mut state = DnbSessionRepository::load_state(&pool, sid, owner)
        .await
        .expect("load state");
    assert_eq!(state.n_level, 2);
    apply_adaptation(&mut state, &trials);
    assert_eq!(state.n_level, 3);
    DnbSessionRepository::update_state(&pool, sid, owner, &state)
        .await
        .expect("update state");
    let reloaded = DnbSessionRepository::load_state(&pool, sid, owner)
        .await
        .expect("reload state");
    assert_eq!(reloaded.n_level, 3);
    assert_eq!(reloaded.n_level_peak, 3);

    // A non-owner cannot read or write the session's state.
    assert!(matches!(
        DnbSessionRepository::load_state(&pool, sid, other).await,
        Err(WisecrowError::Unauthorized)
    ));
    assert!(matches!(
        DnbSessionRepository::update_state(&pool, sid, other, &state).await,
        Err(WisecrowError::Unauthorized)
    ));

    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![owner, other])
        .execute(&pool)
        .await
        .expect("cleanup");
    sqlx::query("DELETE FROM translations WHERE id = $1")
        .bind(tid)
        .execute(&pool)
        .await
        .expect("cleanup translation");
}

/// The web n-back flow must score against server-generated match flags, never a
/// client assertion: trials are inserted with their flags at session start, the
/// client only records responses, and a non-owner cannot record at all.
#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn dnb_scoring_is_server_owned() {
    let pool = test_pool().await;
    let owner = make_user(&pool, "dnb-so@test.local").await;
    let attacker = make_user(&pool, "dnb-so-attacker@test.local").await;

    let en = ensure_lang(&pool, "en", "English").await;
    let ru = ensure_lang(&pool, "ru", "Russian").await;
    let tid: i32 = sqlx::query_scalar(
        "INSERT INTO translations (from_language_id, to_language_id, from_phrase, to_phrase, frequency)
         VALUES ($1, $2, 'server-owned', 'server-owned', 1) RETURNING id",
    )
    .bind(en)
    .bind(ru)
    .fetch_one(&pool)
    .await
    .expect("insert translation");

    let sid: i32 = sqlx::query_scalar(
        "INSERT INTO dnb_sessions (user_id, native_lang, foreign_lang, mode, interval_ms_start, interval_ms_end)
         VALUES ($1, 'ru', 'en', 'audio_written', 4000, 4000) RETURNING id",
    )
    .bind(owner)
    .fetch_one(&pool)
    .await
    .expect("insert dnb session");

    let vocab = DnbVocab {
        translation_id: tid,
        from_phrase: "server-owned".to_owned(),
        to_phrase: "server-owned".to_owned(),
    };
    // Both trials are audio-matches as far as the server is concerned.
    let generated: Vec<Trial> = (1..=2u32)
        .map(|i| Trial {
            trial_number: i,
            n_level: 2,
            audio_vocab: vocab.clone(),
            visual_vocab: vocab.clone(),
            audio_match: true,
            visual_match: true,
            interval_ms: 4000,
        })
        .collect();
    DnbSessionRepository::insert_generated_trials(&pool, sid, owner, &generated)
        .await
        .expect("insert generated trials");

    // Nothing is answered yet, so no trial counts.
    assert!(DnbSessionRepository::load_answered_trials(&pool, sid)
        .await
        .expect("load answered")
        .is_empty());

    let wrong = TrialResponse {
        audio_response: Some(false),
        visual_response: Some(false),
        response_time_ms: None,
    };
    let right = TrialResponse {
        audio_response: Some(true),
        visual_response: Some(true),
        response_time_ms: None,
    };

    // A non-owner cannot record a response into someone else's session.
    assert!(matches!(
        DnbSessionRepository::record_trial_response(&pool, sid, attacker, 1, &right).await,
        Err(WisecrowError::Unauthorized)
    ));

    // The owner records a wrong answer to trial 1 and a right answer to trial 2.
    DnbSessionRepository::record_trial_response(&pool, sid, owner, 1, &wrong)
        .await
        .expect("record trial 1");
    DnbSessionRepository::record_trial_response(&pool, sid, owner, 2, &right)
        .await
        .expect("record trial 2");

    // Accuracy is computed from the server's match flags against the recorded
    // responses: one wrong, one right → 0.5. The client never supplied a flag.
    let answered = DnbSessionRepository::load_answered_trials(&pool, sid)
        .await
        .expect("load answered");
    assert_eq!(answered.len(), 2);
    assert!((channel_accuracy(&answered, Channel::Audio, 2) - 0.5).abs() < f64::EPSILON);

    sqlx::query("DELETE FROM users WHERE id = ANY($1)")
        .bind(vec![owner, attacker])
        .execute(&pool)
        .await
        .expect("cleanup");
    sqlx::query("DELETE FROM translations WHERE id = $1")
        .bind(tid)
        .execute(&pool)
        .await
        .expect("cleanup translation");
}
