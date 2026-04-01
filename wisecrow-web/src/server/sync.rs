use dioxus::prelude::*;

use super::{pool, validate_sync_key};

const SYNC_PAGE_SIZE: i64 = 500;

#[server]
pub async fn sync_languages(
    api_key: String,
    after_id: i32,
) -> Result<Vec<wisecrow_dto::SyncLanguageDto>, ServerFnError> {
    validate_sync_key(&api_key)?;
    let db = pool()?;
    let rows = sqlx::query_as::<_, (i32, String, String)>(
        "SELECT id, code, name FROM languages WHERE id > $1 ORDER BY id LIMIT $2",
    )
    .bind(after_id)
    .bind(SYNC_PAGE_SIZE)
    .fetch_all(db)
    .await
    .map_err(|e| ServerFnError::new(format!("Sync languages failed: {e}")))?;

    Ok(rows
        .into_iter()
        .map(|(id, code, name)| wisecrow_dto::SyncLanguageDto { id, code, name })
        .collect())
}

#[server]
pub async fn sync_translations(
    api_key: String,
    after_id: i32,
) -> Result<Vec<wisecrow_dto::SyncTranslationDto>, ServerFnError> {
    validate_sync_key(&api_key)?;
    let db = pool()?;
    let rows = sqlx::query_as::<_, (i32, String, String, String, String, i32)>(
        "SELECT t.id, fl.code, t.from_phrase, tl.code, t.to_phrase, t.frequency
         FROM translations t
         JOIN languages fl ON fl.id = t.from_language_id
         JOIN languages tl ON tl.id = t.to_language_id
         WHERE t.id > $1 ORDER BY t.id LIMIT $2",
    )
    .bind(after_id)
    .bind(SYNC_PAGE_SIZE)
    .fetch_all(db)
    .await
    .map_err(|e| ServerFnError::new(format!("Sync translations failed: {e}")))?;

    Ok(rows
        .into_iter()
        .map(
            |(id, from_code, from_phrase, to_code, to_phrase, frequency)| {
                wisecrow_dto::SyncTranslationDto {
                    id,
                    from_language_code: from_code,
                    from_phrase,
                    to_language_code: to_code,
                    to_phrase,
                    frequency,
                }
            },
        )
        .collect())
}

#[server]
pub async fn sync_grammar_rules(
    api_key: String,
    after_id: i32,
) -> Result<Vec<wisecrow_dto::SyncGrammarRuleDto>, ServerFnError> {
    validate_sync_key(&api_key)?;
    let db = pool()?;
    let rules = sqlx::query_as::<_, (i32, String, String, String, String, String)>(
        "SELECT gr.id, l.code, cl.code, gr.title, gr.explanation, gr.source
         FROM grammar_rules gr
         JOIN languages l ON l.id = gr.language_id
         JOIN cefr_levels cl ON cl.id = gr.cefr_level_id
         WHERE gr.id > $1 ORDER BY gr.id LIMIT $2",
    )
    .bind(after_id)
    .bind(SYNC_PAGE_SIZE)
    .fetch_all(db)
    .await
    .map_err(|e| ServerFnError::new(format!("Sync grammar rules failed: {e}")))?;

    let mut result = Vec::with_capacity(rules.len());
    for (id, lang_code, cefr_code, title, explanation, source) in rules {
        let examples = sqlx::query_as::<_, (String, Option<String>, bool)>(
            "SELECT sentence, translation, is_correct FROM rule_examples WHERE rule_id = $1 ORDER BY id",
        )
        .bind(id)
        .fetch_all(db)
        .await
        .map_err(|e| ServerFnError::new(format!("Sync rule examples failed: {e}")))?;

        result.push(wisecrow_dto::SyncGrammarRuleDto {
            id,
            language_code: lang_code,
            cefr_level_code: cefr_code,
            title,
            explanation,
            source,
            examples: examples
                .into_iter()
                .map(
                    |(sentence, translation, is_correct)| wisecrow_dto::SyncRuleExampleDto {
                        sentence,
                        translation,
                        is_correct,
                    },
                )
                .collect(),
        });
    }

    Ok(result)
}
