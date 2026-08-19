use chrono::{DateTime, Duration, TimeZone, Utc};
use sqlx::PgPool;
use uuid::Uuid;
use wisecrow::errors::WisecrowError;
use wisecrow::srs::reviews::{ReviewLedger, ReviewSource};
use wisecrow::srs::scheduler::{CardManager, CardState, ReviewRating};
use wisecrow::srs::session::SessionManager;
use wisecrow_dto::{ReviewEventDto, ReviewEventStatusDto, ReviewRatingDto};

type PersistedCard = (
    f32,
    f32,
    i32,
    i32,
    i32,
    i32,
    i16,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
);

#[derive(Clone, Copy)]
struct TestAccount {
    user_id: i32,
    device_id: Uuid,
}

struct TestAccounts {
    canonical: TestAccount,
    reverse: TestAccount,
    repeated: TestAccount,
    outsider: TestAccount,
}

async fn test_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".into());
    let pool = PgPool::connect(&url).await.expect("connect test database");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

async fn clean_fixture(pool: &PgPool) {
    sqlx::query("DELETE FROM users WHERE email LIKE 'review-sync-%'")
        .execute(pool)
        .await
        .expect("clean users");
    sqlx::query("DELETE FROM translations WHERE from_phrase = 'review sync source'")
        .execute(pool)
        .await
        .expect("clean translation");
}

async fn ensure_language(pool: &PgPool, name: &str, code: &str) -> i32 {
    sqlx::query_scalar(
        "INSERT INTO languages (name, code) VALUES ($1, $2)
         ON CONFLICT (code) DO UPDATE SET name = EXCLUDED.name
         RETURNING id",
    )
    .bind(name)
    .bind(code)
    .fetch_one(pool)
    .await
    .expect("ensure language")
}

async fn seed_translation(pool: &PgPool) -> i32 {
    let source = ensure_language(pool, "Review Sync Source", "rv-sync-src").await;
    let target = ensure_language(pool, "Review Sync Target", "rv-sync-dst").await;
    sqlx::query_scalar(
        "INSERT INTO translations (
             from_language_id, from_phrase, to_language_id, to_phrase, frequency
         ) VALUES ($1, 'review sync source', $2, 'review sync target', 10)
         RETURNING id",
    )
    .bind(source)
    .bind(target)
    .fetch_one(pool)
    .await
    .expect("seed translation")
}

async fn seed_account(
    pool: &PgPool,
    label: &str,
    device_id: Uuid,
    translation_id: i32,
    baseline_at: DateTime<Utc>,
) -> TestAccount {
    let email = format!("review-sync-{label}@test.local");
    let user_id =
        sqlx::query_scalar("INSERT INTO users (display_name, email) VALUES ($1, $2) RETURNING id")
            .bind(label)
            .bind(email)
            .fetch_one(pool)
            .await
            .expect("seed user");
    sqlx::query("INSERT INTO mobile_devices (user_id, id, display_name) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(device_id)
        .bind(label)
        .execute(pool)
        .await
        .expect("seed device");
    seed_card_and_baseline(pool, user_id, translation_id, baseline_at).await;
    TestAccount { user_id, device_id }
}

async fn seed_card_and_baseline(
    pool: &PgPool,
    user_id: i32,
    translation_id: i32,
    baseline_at: DateTime<Utc>,
) {
    sqlx::query("INSERT INTO cards (user_id, translation_id, due) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(translation_id)
        .bind(baseline_at)
        .execute(pool)
        .await
        .expect("seed card");
    sqlx::query(
        "INSERT INTO card_review_baselines (
             user_id, translation_id, stability, difficulty, elapsed_days,
             scheduled_days, reps, lapses, state, last_review, due, captured_at
         ) VALUES ($1, $2, 0, 0, 0, 0, 0, 0, 0, NULL, $3, $3)",
    )
    .bind(user_id)
    .bind(translation_id)
    .bind(baseline_at)
    .execute(pool)
    .await
    .expect("capture baseline");
}

fn baseline_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2025, 1, 1, 12, 0, 0)
        .single()
        .expect("valid baseline timestamp")
}

fn fixed_events(translation_id: i32) -> Vec<ReviewEventDto> {
    let baseline = baseline_at();
    [
        (1, ReviewRatingDto::Good, 1),
        (2, ReviewRatingDto::Again, 2),
        (3, ReviewRatingDto::Hard, 3),
        (4, ReviewRatingDto::Easy, 4),
    ]
    .into_iter()
    .map(|(event_id, rating, days)| ReviewEventDto {
        event_id: Uuid::from_u128(event_id),
        translation_id,
        rating,
        occurred_at: baseline + Duration::days(days),
    })
    .collect()
}

async fn load_card(pool: &PgPool, user_id: i32, translation_id: i32) -> PersistedCard {
    sqlx::query_as(
        "SELECT stability, difficulty, elapsed_days, scheduled_days,
                reps, lapses, state, last_review, due
         FROM cards WHERE user_id = $1 AND translation_id = $2",
    )
    .bind(user_id)
    .bind(translation_id)
    .fetch_one(pool)
    .await
    .expect("load card")
}

async fn apply_and_load(
    pool: &PgPool,
    account: TestAccount,
    translation_id: i32,
    events: &[ReviewEventDto],
) -> PersistedCard {
    ReviewLedger::new(pool)
        .apply_batch(
            account.user_id,
            Some(account.device_id),
            ReviewSource::Mobile,
            events,
        )
        .await
        .expect("apply review batch");
    load_card(pool, account.user_id, translation_id).await
}

async fn retry_and_load(pool: &PgPool, account: TestAccount, translation_id: i32) -> PersistedCard {
    let result = ReviewLedger::new(pool)
        .apply_batch(
            account.user_id,
            Some(account.device_id),
            ReviewSource::Mobile,
            &fixed_events(translation_id),
        )
        .await
        .expect("retry review batch");
    assert!(result
        .acknowledgements
        .iter()
        .all(|ack| ack.status == ReviewEventStatusDto::AlreadyApplied));
    load_card(pool, account.user_id, translation_id).await
}

async fn seed_accounts(
    pool: &PgPool,
    translation_id: i32,
    baseline: DateTime<Utc>,
) -> TestAccounts {
    let canonical = seed_account(
        pool,
        "canonical",
        Uuid::from_u128(11),
        translation_id,
        baseline,
    )
    .await;
    let reverse = seed_account(
        pool,
        "reverse",
        Uuid::from_u128(12),
        translation_id,
        baseline,
    )
    .await;
    let repeated = seed_account(
        pool,
        "repeated",
        Uuid::from_u128(13),
        translation_id,
        baseline,
    )
    .await;
    let outsider = seed_account(
        pool,
        "outsider",
        Uuid::from_u128(14),
        translation_id,
        baseline,
    )
    .await;
    TestAccounts {
        canonical,
        reverse,
        repeated,
        outsider,
    }
}

async fn assert_order_and_retry(pool: &PgPool, translation_id: i32, accounts: &TestAccounts) {
    let canonical_card = apply_and_load(
        pool,
        accounts.canonical,
        translation_id,
        &fixed_events(translation_id),
    )
    .await;
    let reversed_events: Vec<_> = fixed_events(translation_id).into_iter().rev().collect();
    let reverse_card =
        apply_and_load(pool, accounts.reverse, translation_id, &reversed_events).await;
    apply_and_load(
        pool,
        accounts.repeated,
        translation_id,
        &fixed_events(translation_id),
    )
    .await;
    let repeated_card = retry_and_load(pool, accounts.repeated, translation_id).await;
    assert_eq!(canonical_card, reverse_card);
    assert_eq!(canonical_card, repeated_card);

    let repeated_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM review_events WHERE user_id = $1")
            .bind(accounts.repeated.user_id)
            .fetch_one(pool)
            .await
            .expect("count repeated events");
    assert_eq!(repeated_count, 4);
}

async fn assert_collision_and_scope(pool: &PgPool, translation_id: i32, accounts: &TestAccounts) {
    let mut collision = fixed_events(translation_id).remove(0);
    collision.rating = ReviewRatingDto::Easy;
    assert!(matches!(
        ReviewLedger::new(pool)
            .apply_batch(
                accounts.repeated.user_id,
                Some(accounts.repeated.device_id),
                ReviewSource::Mobile,
                &[collision]
            )
            .await,
        Err(WisecrowError::Conflict(_))
    ));
    assert!(matches!(
        ReviewLedger::new(pool)
            .apply_batch(
                accounts.outsider.user_id,
                Some(accounts.repeated.device_id),
                ReviewSource::Mobile,
                &fixed_events(translation_id)
            )
            .await,
        Err(WisecrowError::Unauthorized)
    ));
}

async fn assert_validation_bounds(pool: &PgPool, translation_id: i32, account: TestAccount) {
    let event = |event_id, occurred_at| ReviewEventDto {
        event_id: Uuid::from_u128(event_id),
        translation_id,
        rating: ReviewRatingDto::Good,
        occurred_at,
    };
    let ledger = ReviewLedger::new(pool);
    assert!(matches!(
        ledger
            .apply_batch(
                account.user_id,
                Some(account.device_id),
                ReviewSource::Mobile,
                &[event(101, baseline_at() - Duration::seconds(1))]
            )
            .await,
        Err(WisecrowError::InvalidInput(_))
    ));
    assert!(matches!(
        ledger
            .apply_batch(
                account.user_id,
                Some(account.device_id),
                ReviewSource::Mobile,
                &[event(102, Utc::now() + Duration::minutes(6))]
            )
            .await,
        Err(WisecrowError::InvalidInput(_))
    ));
    let oversized: Vec<_> = (1_000_u128..1_501)
        .map(|event_id| event(event_id, baseline_at() + Duration::days(1)))
        .collect();
    assert!(matches!(
        ledger
            .apply_batch(
                account.user_id,
                Some(account.device_id),
                ReviewSource::Mobile,
                &oversized
            )
            .await,
        Err(WisecrowError::InvalidInput(_))
    ));
}

async fn seed_session(
    pool: &PgPool,
    account: TestAccount,
    translation_id: i32,
) -> (i32, CardState) {
    let session_id: i32 = sqlx::query_scalar(
        "INSERT INTO sessions (
             user_id, native_lang, foreign_lang, deck_size, speed_ms
         ) VALUES ($1, 'rv-sync-dst', 'rv-sync-src', 1, 3000)
         RETURNING id",
    )
    .bind(account.user_id)
    .fetch_one(pool)
    .await
    .expect("seed session");
    let card = CardManager::card_for_translation(pool, translation_id, account.user_id)
        .await
        .expect("load session card")
        .expect("session card exists");
    sqlx::query("INSERT INTO session_cards (session_id, card_id, position) VALUES ($1, $2, 0)")
        .bind(session_id)
        .bind(card.card_id)
        .execute(pool)
        .await
        .expect("seed session card");
    (session_id, card)
}

async fn assert_web_session_success(pool: &PgPool, account: TestAccount, translation_id: i32) {
    let (session_id, card) = seed_session(pool, account, translation_id).await;
    SessionManager::answer_card(pool, session_id, account.user_id, &card, ReviewRating::Good)
        .await
        .expect("apply web review");
    let web_event: (Option<Uuid>, String, i16) =
        sqlx::query_as("SELECT device_id, source, rating FROM review_events WHERE user_id = $1")
            .bind(account.user_id)
            .fetch_one(pool)
            .await
            .expect("load web event");
    assert_eq!(web_event, (None, "web".into(), 3));
}

async fn assert_web_session_rollback(pool: &PgPool, account: TestAccount, translation_id: i32) {
    let (session_id, card) = seed_session(pool, account, translation_id).await;
    assert!(matches!(
        SessionManager::answer_card(pool, session_id, account.user_id, &card, ReviewRating::Good)
            .await,
        Err(WisecrowError::InvalidInput(_))
    ));
    let answered: bool = sqlx::query_scalar(
        "SELECT answered FROM session_cards WHERE session_id = $1 AND card_id = $2",
    )
    .bind(session_id)
    .bind(card.card_id)
    .fetch_one(pool)
    .await
    .expect("load rejected session card");
    assert!(!answered);
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn review_batches_are_order_independent_idempotent_and_user_scoped() {
    let pool = test_pool().await;
    clean_fixture(&pool).await;
    let translation_id = seed_translation(&pool).await;
    let accounts = seed_accounts(&pool, translation_id, baseline_at()).await;
    assert_order_and_retry(&pool, translation_id, &accounts).await;
    assert_collision_and_scope(&pool, translation_id, &accounts).await;
    assert_validation_bounds(&pool, translation_id, accounts.canonical).await;
    let account = seed_account(
        &pool,
        "web",
        Uuid::from_u128(21),
        translation_id,
        baseline_at(),
    )
    .await;
    assert_web_session_success(&pool, account, translation_id).await;

    let future = Utc::now() + Duration::hours(1);
    let rejected = seed_account(
        &pool,
        "web-rejected",
        Uuid::from_u128(22),
        translation_id,
        future,
    )
    .await;
    assert_web_session_rollback(&pool, rejected, translation_id).await;
    clean_fixture(&pool).await;
}
