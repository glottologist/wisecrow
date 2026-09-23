//! End-to-end behaviour of the `/api/grammar/*` routes.
//!
//! The properties worth holding here are the ones a unit test cannot see: that
//! a served item crosses the wire without its answer, that the verdict comes
//! back from the server's own regrading, and that a language whose bank has
//! not been promoted reports an empty bank rather than an error.

#![cfg(feature = "server")]

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use sqlx::types::chrono::Utc;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wisecrow::auth::hash_password;
use wisecrow_dto::{BrainmapDto, GrammarSessionDto, MasteryBandDto, VerdictDto};
use wisecrow_web::server::auth::issue_session;
use wisecrow_web::server::{build_router, init_pool, pool};

const EMAIL: &str = "grammar-routes@test.local";
const SLUG_PREFIX: &str = "grammar-routes-";
const ANSWER: &str = "estoy";

async fn post(path: &str, request: Value, token: &str) -> Response {
    let request = Request::post(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, ["Bearer ", token].concat())
        .body(Body::from(request.to_string()))
        .expect("request");
    build_router().oneshot(request).await.expect("response")
}

async fn response_json<T: DeserializeOwned>(response: Response, label: &str) -> T {
    assert_eq!(response.status(), StatusCode::OK, "{label}");
    let body = to_bytes(response.into_body(), 256 * 1024)
        .await
        .expect(label);
    serde_json::from_slice(&body).unwrap_or_else(|error| {
        panic!("{label}: {error}; body: {}", String::from_utf8_lossy(&body))
    })
}

/// Empties the Spanish syllabus and this suite's learner.
///
/// Every grammar point of the language goes, not just the ones seeded here:
/// the empty-bank case asserts that *nothing* is promoted, and the database is
/// shared with suites that leave Spanish points behind. The integration runner
/// serialises tests, so this cannot pull the rug from under another.
async fn cleanup(db: &PgPool) {
    sqlx::query("DELETE FROM users WHERE email = $1")
        .bind(EMAIL)
        .execute(db)
        .await
        .expect("user cleanup");
    sqlx::query(
        "DELETE FROM grammar_rules gr USING languages l
         WHERE l.id = gr.language_id AND l.code = 'es'",
    )
    .execute(db)
    .await
    .expect("rule cleanup");
}

async fn seed_rule(db: &PgPool, slug: &str, status: &str) -> i32 {
    let language_id: i32 = sqlx::query_scalar(
        "INSERT INTO languages (code, name) VALUES ('es', 'Spanish')
         ON CONFLICT (code) DO UPDATE SET name = EXCLUDED.name RETURNING id",
    )
    .fetch_one(db)
    .await
    .expect("language");

    let rule_id: i32 = sqlx::query_scalar(
        "INSERT INTO grammar_rules (language_id, cefr_level_id, slug, title, explanation, source)
         SELECT $1, cl.id, $2, 'Ser vs estar', 'Permanent against temporary.', 'reference'
         FROM cefr_levels cl WHERE cl.code = 'A1'
         RETURNING id",
    )
    .bind(language_id)
    .bind(slug)
    .fetch_one(db)
    .await
    .expect("rule");

    sqlx::query(
        "INSERT INTO quiz_items (rule_id, kind, prompt, answer, accepted, status, content_sha256)
         VALUES ($1, 'cloze', 'Yo ___ cansado.', $2, '[]'::jsonb, $3, $4)",
    )
    .bind(rule_id)
    .bind(ANSWER)
    .bind(status)
    .bind(format!("{:0>64}", format!("{rule_id:x}")))
    .execute(db)
    .await
    .expect("item");

    rule_id
}

#[tokio::test]
#[ignore = "requires PostgreSQL"]
async fn grammar_routes_serve_regrade_and_report() {
    std::env::set_var(
        "WISECROW__DB_URL",
        std::env::var("TEST_DATABASE_URL")
            .unwrap_or_else(|_| "postgres://wisecrow:wisecrow@localhost:5433/wisecrow_test".into()),
    );
    init_pool().await.expect("init pool");
    let db = pool().expect("pool");
    cleanup(db).await;

    let hash = hash_password("hunter2").expect("hash");
    let user_id: i32 = sqlx::query_scalar(
        "INSERT INTO users (display_name, email, password_hash, is_admin)
         VALUES ('Grammar', $1, $2, false) RETURNING id",
    )
    .bind(EMAIL)
    .bind(&hash)
    .fetch_one(db)
    .await
    .expect("user");
    let token = issue_session(db, user_id).await.expect("session");

    // A bank holding nothing promoted is an ordinary state, not a failure.
    seed_rule(db, &format!("{SLUG_PREFIX}ungated"), "candidate").await;
    let empty: Option<GrammarSessionDto> = response_json(
        post(
            "/api/grammar/session/start",
            json!({"native": "en", "foreign": "es", "level": null}),
            &token,
        )
        .await,
        "empty bank",
    )
    .await;
    assert!(empty.is_none(), "an unpromoted bank yields no session");

    let rule_id = seed_rule(db, &format!("{SLUG_PREFIX}ready"), "active").await;
    let session: GrammarSessionDto = response_json(
        post(
            "/api/grammar/session/start",
            json!({"native": "en", "foreign": "es", "level": null}),
            &token,
        )
        .await,
        "session start",
    )
    .await;
    let item = session
        .items
        .iter()
        .find(|item| item.rule_id == rule_id)
        .expect("the promoted item is served");
    assert!(
        !session
            .items
            .iter()
            .any(|item| item.prompt.contains(ANSWER)),
        "a served item must not carry its own answer"
    );

    let submit = |answer: &str, event_id: Uuid| {
        let body = json!({"submission": {
            "session_id": session.session_id,
            "event_id": event_id,
            "item_id": item.item_id,
            "revision": item.revision,
            "answer": answer,
            "chose_option": false,
            "hint_shown": false,
            "ordinal": 1,
            "occurred_at": Utc::now(),
        }});
        post("/api/grammar/session/submit", body, &token)
    };

    // The server regrades: case folding is its business, not the client's.
    let wrong_event = Uuid::new_v4();
    let wrong: VerdictDto = response_json(submit("nope", wrong_event).await, "wrong answer").await;
    assert_eq!(wrong.event_id, wrong_event);
    assert!(!wrong.correct);

    let right: VerdictDto =
        response_json(submit("ESTOY", Uuid::new_v4()).await, "right answer").await;
    assert!(right.correct, "the grader folds case beyond ASCII");

    let brainmap: BrainmapDto = response_json(
        post(
            "/api/grammar/brainmap",
            json!({"native": "en", "foreign": "es"}),
            &token,
        )
        .await,
        "brainmap",
    )
    .await;
    let cells: Vec<_> = brainmap
        .cells
        .iter()
        .filter(|cell| cell.slug.starts_with(SLUG_PREFIX))
        .collect();
    assert_eq!(
        cells.len(),
        2,
        "every syllabus point appears, bank or no bank"
    );

    let attempted = cells
        .iter()
        .find(|cell| cell.rule_id == rule_id)
        .expect("the attempted point");
    assert_eq!(attempted.attempts, 2);
    assert_eq!(attempted.provenance, "reference");
    assert!(
        matches!(
            attempted.band,
            MasteryBandDto::ProvisionalRed
                | MasteryBandDto::ProvisionalAmber
                | MasteryBandDto::ProvisionalGreen
        ),
        "two attempts cannot settle a band: {:?}",
        attempted.band
    );

    let untouched = cells
        .iter()
        .find(|cell| cell.rule_id != rule_id)
        .expect("the untouched point");
    assert_eq!(untouched.band, MasteryBandDto::Unseen);
    assert_eq!(untouched.accuracy, None);

    let completed = post(
        "/api/grammar/session/complete",
        json!({"session_id": session.session_id}),
        &token,
    )
    .await;
    assert_eq!(completed.status(), StatusCode::OK);

    let refused = submit("ESTOY", Uuid::new_v4()).await;
    assert_ne!(
        refused.status(),
        StatusCode::OK,
        "a closed session accepts nothing further"
    );

    cleanup(db).await;
}
