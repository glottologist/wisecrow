//! Word classes for the top of a pair's corpus.

use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::frequency::MATCH_TRIM_SQL;
use crate::llm::cache::CachedLlm;
use crate::llm::prompts::{word_class_prompt, WordClassInput};

/// Distinct forms considered for classification, most frequent first.
pub const WORD_CLASS_WINDOW: i64 = 2000;
/// Forms one call sends.
pub const WORD_CLASS_BATCH: i64 = 100;
/// Bump only when a stored class should be recomputed. Rows already stored
/// are not rewritten by this pass.
pub const CLASSIFIER_VERSION: i32 = 1;
const WORD_CLASS_TOKENS: u32 = 4096;
const LEMMA_MAX: usize = 64;

/// A class stored on `word_classes.class`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WordClass {
    Verb,
    Noun,
    Adjective,
    Adverb,
    Pronoun,
    Preposition,
    Conjunction,
    Determiner,
    Numeral,
    Interjection,
    Other,
}

impl WordClass {
    /// The value stored in `word_classes.class`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verb => "verb",
            Self::Noun => "noun",
            Self::Adjective => "adjective",
            Self::Adverb => "adverb",
            Self::Pronoun => "pronoun",
            Self::Preposition => "preposition",
            Self::Conjunction => "conjunction",
            Self::Determiner => "determiner",
            Self::Numeral => "numeral",
            Self::Interjection => "interjection",
            Self::Other => "other",
        }
    }

    /// Parses a label. An unknown label is `None`, and the caller stores `other`.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "verb" => Self::Verb,
            "noun" => Self::Noun,
            "adjective" => Self::Adjective,
            "adverb" => Self::Adverb,
            "pronoun" => Self::Pronoun,
            "preposition" => Self::Preposition,
            "conjunction" => Self::Conjunction,
            "determiner" => Self::Determiner,
            "numeral" => Self::Numeral,
            "interjection" => Self::Interjection,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

#[derive(Debug, serde::Deserialize)]
struct ClassBatch {
    classes: Vec<ClassLabel>,
}

#[derive(Debug, serde::Deserialize)]
struct ClassLabel {
    word: String,
    class: String,
    lemma: String,
}

/// Classifies one batch of forms from the top of the pair's evidence.
///
/// Returns how many rows this call wrote. A form already stored is not sent.
///
/// # Errors
///
/// Returns [`WisecrowError::LlmUnavailable`] when the model cannot be asked,
/// [`WisecrowError::InvalidInput`] for a malformed language code, and a
/// database error when a read or write fails. An omitted or unknown class is
/// stored as `other` and is not an error.
pub async fn classify_words(
    pool: &PgPool,
    llm: &CachedLlm<'_>,
    native_lang: &str,
    foreign_lang: &str,
) -> Result<usize, WisecrowError> {
    if !crate::lang::is_valid_code(native_lang) || !crate::lang::is_valid_code(foreign_lang) {
        return Err(WisecrowError::InvalidInput(format!(
            "Invalid language code: {native_lang}/{foreign_lang}"
        )));
    }
    let foreign_name: Option<String> =
        sqlx::query_scalar("SELECT name FROM languages WHERE code = $1")
            .bind(foreign_lang)
            .fetch_optional(pool)
            .await?;
    let Some(foreign_name) = foreign_name else {
        return Err(WisecrowError::InvalidInput(format!(
            "Unknown language: {foreign_lang}"
        )));
    };
    let sql = format!(
        "WITH ranked AS (
             SELECT lower(btrim(t.to_phrase, '{trim}')) AS word,
                    max(t.corpus_frequency) AS freq
             FROM corpus_evidence_translations t
             JOIN languages native ON native.id = t.from_language_id
             JOIN languages \"foreign\" ON \"foreign\".id = t.to_language_id
             WHERE native.code = $1 AND \"foreign\".code = $2
             GROUP BY 1
         ),
         windowed AS (
             SELECT word, freq
             FROM ranked
             WHERE char_length(word) BETWEEN 1 AND 64
               AND word !~ '[[:space:]]'
             ORDER BY freq DESC NULLS LAST, word
             LIMIT $3
         )
         SELECT windowed.word
         FROM windowed
         WHERE NOT EXISTS (
             SELECT 1
             FROM word_classes wc
             JOIN languages l ON l.id = wc.language_id
             WHERE l.code = $2 AND wc.word = windowed.word
         )
         ORDER BY windowed.freq DESC NULLS LAST, windowed.word
         LIMIT $4",
        trim = MATCH_TRIM_SQL
    );
    let words: Vec<String> = sqlx::query_scalar(&sql)
        .bind(native_lang)
        .bind(foreign_lang)
        .bind(WORD_CLASS_WINDOW)
        .bind(WORD_CLASS_BATCH)
        .fetch_all(pool)
        .await?;
    if words.is_empty() {
        return Ok(0);
    }
    let inputs: Vec<WordClassInput<'_>> =
        words.iter().map(|word| WordClassInput { word }).collect();
    let prompt = word_class_prompt(&foreign_name, &inputs)?;
    let answer = match llm
        .generate(&prompt, WORD_CLASS_TOKENS, parse_classes, |_| true)
        .await
    {
        Ok(answer) => answer,
        Err(error) => {
            let error = WisecrowError::from(error);
            if matches!(error, WisecrowError::LlmUnavailable(_)) {
                return Err(error);
            }
            let quoted = format!(
                "{prompt}\n\nThe previous answer was rejected: {error}\n\
                 Return only the JSON object."
            );
            match llm
                .generate(&quoted, WORD_CLASS_TOKENS, parse_classes, |_| true)
                .await
            {
                Ok(answer) => answer,
                Err(error) => {
                    let error = WisecrowError::from(error);
                    if matches!(error, WisecrowError::LlmUnavailable(_)) {
                        return Err(error);
                    }
                    ClassBatch {
                        classes: Vec::new(),
                    }
                }
            }
        }
    };
    let mut labels = std::collections::HashMap::<String, (WordClass, String)>::new();
    for label in answer.classes {
        let Some(class) = WordClass::parse(&label.class) else {
            continue;
        };
        let lemma = if label.lemma.is_empty() || label.lemma.chars().count() > LEMMA_MAX {
            label.word.clone()
        } else {
            label.lemma
        };
        labels.insert(label.word, (class, lemma));
    }
    let mut forms = Vec::with_capacity(words.len());
    let mut classes = Vec::with_capacity(words.len());
    let mut lemmas = Vec::with_capacity(words.len());
    for word in &words {
        let (class, lemma) = labels
            .remove(word)
            .unwrap_or_else(|| (WordClass::Other, word.clone()));
        forms.push(word.clone());
        classes.push(class.as_str().to_owned());
        lemmas.push(lemma);
    }
    let written = sqlx::query(
        "INSERT INTO word_classes
             (language_id, word, class, lemma, classifier_version)
         SELECT l.id, batch.word, batch.class, batch.lemma, $4
         FROM languages l
         JOIN unnest($1::text[], $2::text[], $3::text[]) AS batch(word, class, lemma)
           ON TRUE
         WHERE l.code = $5
         ON CONFLICT (language_id, word) DO NOTHING",
    )
    .bind(&forms)
    .bind(&classes)
    .bind(&lemmas)
    .bind(CLASSIFIER_VERSION)
    .bind(foreign_lang)
    .execute(pool)
    .await?;
    usize::try_from(written.rows_affected())
        .map_err(|_| WisecrowError::InvalidInput("class insert count does not fit".into()))
}

fn parse_classes(response: &str) -> Result<ClassBatch, WisecrowError> {
    crate::llm::parse_fenced_json(response, "word classes")
}
