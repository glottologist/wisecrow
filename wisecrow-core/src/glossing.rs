//! Canonical user-facing presentations for frequency-ranked word cards.

use std::collections::HashMap;

use crate::errors::WisecrowError;
use crate::llm::LlmProvider;
use crate::presentation::CURRENT_PRESENTATION_VERSION;
use sqlx::PgPool;

const PRESENTATION_BATCH: usize = 50;
const PRESENTATION_MAX_TOKENS: u32 = 8192;
const MAX_PRESENTATION_CHARS: usize = 200;

/// A normalised word and representative corpus spelling awaiting enrichment.
#[derive(Debug, PartialEq, Eq)]
pub struct PresentationCandidate {
    /// Normalised key used by deck and presentation lookups.
    pub word: String,
    /// Representative corpus spelling supplied to the enrichment model.
    pub surface: String,
}

#[derive(Debug, serde::Deserialize)]
struct PresentationEntry {
    word: String,
    display_form: String,
    translation: String,
    teachable: bool,
    image_query: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct PresentationResponse {
    presentations: Vec<PresentationEntry>,
}

struct ValidatedPresentation<'a> {
    word: &'a str,
    display_form: String,
    translation: String,
    teachable: bool,
    image_query: Option<String>,
}

struct PresentationColumns<'a> {
    words: Vec<&'a str>,
    displays: Vec<&'a str>,
    translations: Vec<&'a str>,
    teachable: Vec<bool>,
    images: Vec<Option<&'a str>>,
}

impl<'a> PresentationColumns<'a> {
    fn from_presentations(presentations: &'a [ValidatedPresentation<'_>]) -> Self {
        Self {
            words: presentations.iter().map(|item| item.word).collect(),
            displays: presentations
                .iter()
                .map(|item| item.display_form.as_str())
                .collect(),
            translations: presentations
                .iter()
                .map(|item| item.translation.as_str())
                .collect(),
            teachable: presentations.iter().map(|item| item.teachable).collect(),
            images: presentations
                .iter()
                .map(|item| item.image_query.as_deref())
                .collect(),
        }
    }
}

/// Returns highest-frequency words whose presentation predates the current contract.
///
/// # Errors
///
/// Returns an error when the database query fails.
pub async fn pending_presentations(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
    limit: u32,
) -> Result<Vec<PresentationCandidate>, WisecrowError> {
    let rows = sqlx::query_as::<_, (String, String)>(&pending_statement())
        .bind(native_lang)
        .bind(foreign_lang)
        .bind(CURRENT_PRESENTATION_VERSION)
        .bind(i64::from(limit))
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(word, surface)| PresentationCandidate { word, surface })
        .collect())
}

fn pending_statement() -> String {
    format!(
        "SELECT best.norm_to, best.to_phrase
         FROM (
           SELECT DISTINCT ON (norm_to)
                  id, to_phrase, frequency, norm_to
           FROM (
             SELECT t.id, t.to_phrase, t.corpus_frequency AS frequency,
                    lower(btrim(t.to_phrase, '{trim}')) AS norm_to
             FROM translations t
             JOIN languages fl ON fl.id = t.from_language_id
             JOIN languages tl ON tl.id = t.to_language_id
             WHERE fl.code = $1 AND tl.code = $2
               AND t.corpus_frequency > 1
               AND LENGTH(t.from_phrase) BETWEEN 2 AND 200
               AND LENGTH(t.to_phrase) BETWEEN 2 AND 200
               AND NOT EXISTS (
                 SELECT 1 FROM phrase_translations pt
                 WHERE pt.translation_id = t.id
               )
           ) candidates
           ORDER BY norm_to, frequency DESC, LENGTH(to_phrase), id
         ) best
         LEFT JOIN word_glosses g
           ON g.lang_code = $2 AND g.native_lang = $1 AND g.word = best.norm_to
         WHERE COALESCE(g.presentation_version, 0) < $3
         ORDER BY best.frequency DESC, best.norm_to, best.id
         LIMIT $4",
        trim = crate::frequency::MATCH_TRIM_SQL
    )
}

/// Generates and stores current canonical presentations for `candidates`.
///
/// # Errors
///
/// Returns an error when prompt encoding, generation, response parsing, or storage fails.
pub async fn enrich_presentations(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    candidates: &[PresentationCandidate],
    native_lang: &str,
    foreign_lang: &str,
    native_lang_name: &str,
    foreign_lang_name: &str,
) -> Result<usize, WisecrowError> {
    let mut written = 0usize;
    for batch in candidates.chunks(PRESENTATION_BATCH) {
        let batch_written = enrich_batch(
            pool,
            provider,
            batch,
            native_lang,
            foreign_lang,
            native_lang_name,
            foreign_lang_name,
        )
        .await?;
        written = written.saturating_add(batch_written);
    }
    Ok(written)
}

async fn enrich_batch(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    candidates: &[PresentationCandidate],
    native_lang: &str,
    foreign_lang: &str,
    native_lang_name: &str,
    foreign_lang_name: &str,
) -> Result<usize, WisecrowError> {
    let words: Vec<(&str, &str)> = candidates
        .iter()
        .map(|candidate| (candidate.word.as_str(), candidate.surface.as_str()))
        .collect();
    let prompt = crate::llm::prompts::deck_presentations_prompt(
        &words,
        foreign_lang_name,
        native_lang_name,
    )?;
    let response = provider.generate(&prompt, PRESENTATION_MAX_TOKENS).await?;
    let parsed = crate::llm::parse_fenced_json(&response, "word-presentation JSON")?;
    let presentations = pair_presentations(candidates, parsed);
    store_presentations(pool, native_lang, foreign_lang, &presentations).await
}

async fn store_presentations(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
    presentations: &[ValidatedPresentation<'_>],
) -> Result<usize, WisecrowError> {
    if presentations.is_empty() {
        return Ok(0);
    }
    let columns = PresentationColumns::from_presentations(presentations);
    let result = upsert_presentations(pool, native_lang, foreign_lang, &columns).await?;
    usize::try_from(result.rows_affected())
        .map_err(|_| WisecrowError::InvalidInput("presentation write count overflow".into()))
}

async fn upsert_presentations(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
    columns: &PresentationColumns<'_>,
) -> Result<sqlx::postgres::PgQueryResult, WisecrowError> {
    let result = sqlx::query(
        "INSERT INTO word_glosses
             (lang_code, word, native_lang, translation, display_form,
              teachable, image_query, presentation_version)
         SELECT $1, p.word, $2, p.translation, p.display_form,
                p.teachable, p.image_query, $3
         FROM UNNEST($4::TEXT[], $5::TEXT[], $6::TEXT[], $7::BOOLEAN[], $8::TEXT[])
              AS p(word, display_form, translation, teachable, image_query)
         ON CONFLICT (lang_code, word, native_lang) DO UPDATE
         SET translation = EXCLUDED.translation,
             display_form = EXCLUDED.display_form,
             teachable = EXCLUDED.teachable,
             image_query = EXCLUDED.image_query,
             presentation_version = EXCLUDED.presentation_version,
             created_at = CURRENT_TIMESTAMP",
    )
    .bind(foreign_lang)
    .bind(native_lang)
    .bind(CURRENT_PRESENTATION_VERSION)
    .bind(&columns.words)
    .bind(&columns.displays)
    .bind(&columns.translations)
    .bind(&columns.teachable)
    .bind(&columns.images)
    .execute(pool)
    .await?;
    Ok(result)
}

fn pair_presentations<'a>(
    requested: &'a [PresentationCandidate],
    parsed: PresentationResponse,
) -> Vec<ValidatedPresentation<'a>> {
    let mut offered = HashMap::new();
    for entry in parsed.presentations {
        let word = crate::lang::normalise_for_match(&entry.word);
        offered.entry(word).or_insert(entry);
    }
    requested
        .iter()
        .filter_map(|candidate| {
            offered
                .remove(&candidate.word)
                .and_then(|entry| validate_presentation(candidate, entry))
        })
        .collect()
}

fn validate_presentation<'a>(
    candidate: &'a PresentationCandidate,
    entry: PresentationEntry,
) -> Option<ValidatedPresentation<'a>> {
    let display_form = clean_required(entry.display_form)?;
    let translation = clean_required(entry.translation)?;
    let image_query = clean_optional(entry.image_query)?;
    Some(ValidatedPresentation {
        word: &candidate.word,
        display_form,
        translation,
        teachable: entry.teachable,
        image_query: entry.teachable.then_some(image_query).flatten(),
    })
}

fn clean_required(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()
        && trimmed.chars().count() <= MAX_PRESENTATION_CHARS
        && !crate::lang::has_invisible_chars(trimmed))
    .then(|| String::from(trimmed))
}

fn clean_optional(value: Option<String>) -> Option<Option<String>> {
    let Some(value) = value else {
        return Some(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Some(None);
    }
    (trimmed.chars().count() <= MAX_PRESENTATION_CHARS
        && !crate::lang::has_invisible_chars(trimmed))
    .then(|| Some(String::from(trimmed)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn candidate() -> PresentationCandidate {
        PresentationCandidate {
            word: "chien".to_owned(),
            surface: "Chien.".to_owned(),
        }
    }

    fn response(
        word: &str,
        display_form: &str,
        translation: &str,
        teachable: bool,
        image_query: Option<&str>,
    ) -> PresentationResponse {
        PresentationResponse {
            presentations: vec![PresentationEntry {
                word: word.to_owned(),
                display_form: display_form.to_owned(),
                translation: translation.to_owned(),
                teachable,
                image_query: image_query.map(str::to_owned),
            }],
        }
    }

    #[rstest]
    #[case(
        "Chien.",
        " chien ",
        " dog ",
        true,
        Some(" friendly dog "),
        Some(("chien", "dog", true, Some("friendly dog")))
    )]
    #[case("chat", "chat", "cat", true, Some("cat"), None)]
    #[case("chien", "chien", "   ", true, Some("dog"), None)]
    #[case(
        "chien",
        "chien",
        "dog",
        false,
        Some("dog"),
        Some(("chien", "dog", false, None))
    )]
    #[case(
        "chien",
        "chien",
        "dog",
        true,
        Some("   "),
        Some(("chien", "dog", true, None))
    )]
    fn responses_are_validated_against_requested_keys(
        #[case] word: &str,
        #[case] display_form: &str,
        #[case] translation: &str,
        #[case] teachable: bool,
        #[case] image_query: Option<&str>,
        #[case] expected: Option<(&str, &str, bool, Option<&str>)>,
    ) {
        let requested = [candidate()];
        let validated = pair_presentations(
            &requested,
            response(word, display_form, translation, teachable, image_query),
        );
        let actual = validated.first().map(|item| {
            (
                item.display_form.as_str(),
                item.translation.as_str(),
                item.teachable,
                item.image_query.as_deref(),
            )
        });
        assert_eq!(actual, expected);
    }

    #[test]
    fn overlong_fields_are_rejected() {
        let requested = [candidate()];
        let parsed = response("chien", &"x".repeat(201), "dog", true, None);
        assert!(pair_presentations(&requested, parsed).is_empty());
    }
}
