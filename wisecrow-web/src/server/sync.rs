//! Remote→local corpus sync: authenticated `GET` endpoints the core `SyncClient`
//! pulls from (`GET /api/sync_*` + `x-api-key` header + `after_id` query). Uses
//! per-client revocable keys (constant-time), falling back to the legacy single
//! `WISECROW__SYNC_API_KEY` while no per-client keys are provisioned.

use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use sqlx::PgPool;

use wisecrow::auth::verify_key_ct;
use wisecrow::sync::clients::SyncClientRepository;
use wisecrow_dto::{SyncGrammarRuleDto, SyncLanguageDto, SyncRuleExampleDto, SyncTranslationDto};

use super::{pool, SYNC_API_KEY};

const SYNC_PAGE_SIZE: i64 = 500;

#[derive(Deserialize)]
struct AfterId {
    #[serde(default)]
    after_id: i32,
}

/// Routes for the corpus sync API, merged into the fullstack router.
pub fn sync_routes() -> Router {
    Router::new()
        .route("/api/sync_languages", get(sync_languages))
        .route("/api/sync_translations", get(sync_translations))
        .route("/api/sync_grammar_rules", get(sync_grammar_rules))
}

fn db_or_500() -> Result<&'static PgPool, StatusCode> {
    pool().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Authenticates a sync request: a non-revoked per-client key (constant-time),
/// or the legacy single shared key as a fallback.
async fn authenticate(db: &PgPool, headers: &HeaderMap) -> Result<(), StatusCode> {
    let key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if SyncClientRepository::verify(db, key)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    {
        return Ok(());
    }
    if let Some(Some(expected)) = SYNC_API_KEY.get() {
        if verify_key_ct(expected.as_bytes(), key.as_bytes()) {
            return Ok(());
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}

async fn sync_languages(
    headers: HeaderMap,
    Query(q): Query<AfterId>,
) -> Result<Json<Vec<SyncLanguageDto>>, StatusCode> {
    let db = db_or_500()?;
    authenticate(db, &headers).await?;
    let rows = sqlx::query_as::<_, (i32, String, String)>(
        "SELECT id, code, name FROM languages WHERE id > $1 ORDER BY id LIMIT $2",
    )
    .bind(q.after_id)
    .bind(SYNC_PAGE_SIZE)
    .fetch_all(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        rows.into_iter()
            .map(|(id, code, name)| SyncLanguageDto { id, code, name })
            .collect(),
    ))
}

async fn sync_translations(
    headers: HeaderMap,
    Query(q): Query<AfterId>,
) -> Result<Json<Vec<SyncTranslationDto>>, StatusCode> {
    let db = db_or_500()?;
    authenticate(db, &headers).await?;
    let rows = sqlx::query_as::<_, (i32, String, String, String, String, i32)>(
        "SELECT t.id, fl.code, t.from_phrase, tl.code, t.to_phrase, t.frequency
         FROM translations t
         JOIN languages fl ON fl.id = t.from_language_id
         JOIN languages tl ON tl.id = t.to_language_id
         WHERE t.id > $1 ORDER BY t.id LIMIT $2",
    )
    .bind(q.after_id)
    .bind(SYNC_PAGE_SIZE)
    .fetch_all(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        rows.into_iter()
            .map(
                |(id, from_code, from_phrase, to_code, to_phrase, frequency)| SyncTranslationDto {
                    id,
                    from_language_code: from_code,
                    from_phrase,
                    to_language_code: to_code,
                    to_phrase,
                    frequency,
                },
            )
            .collect(),
    ))
}

async fn sync_grammar_rules(
    headers: HeaderMap,
    Query(q): Query<AfterId>,
) -> Result<Json<Vec<SyncGrammarRuleDto>>, StatusCode> {
    let db = db_or_500()?;
    authenticate(db, &headers).await?;
    let rules = sqlx::query_as::<_, (i32, String, String, String, String, String)>(
        "SELECT gr.id, l.code, cl.code, gr.title, gr.explanation, gr.source
         FROM grammar_rules gr
         JOIN languages l ON l.id = gr.language_id
         JOIN cefr_levels cl ON cl.id = gr.cefr_level_id
         WHERE gr.id > $1 ORDER BY gr.id LIMIT $2",
    )
    .bind(q.after_id)
    .bind(SYNC_PAGE_SIZE)
    .fetch_all(db)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut result = Vec::with_capacity(rules.len());
    for (id, lang_code, cefr_code, title, explanation, source) in rules {
        let examples = sqlx::query_as::<_, (String, Option<String>, bool)>(
            "SELECT sentence, translation, is_correct FROM rule_examples WHERE rule_id = $1 ORDER BY id",
        )
        .bind(id)
        .fetch_all(db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

        result.push(SyncGrammarRuleDto {
            id,
            language_code: lang_code,
            cefr_level_code: cefr_code,
            title,
            explanation,
            source,
            examples: examples
                .into_iter()
                .map(|(sentence, translation, is_correct)| SyncRuleExampleDto {
                    sentence,
                    translation,
                    is_correct,
                })
                .collect(),
        });
    }
    Ok(Json(result))
}
