use sqlx::PgPool;

use crate::errors::WisecrowError;

/// Version of the canonical word-presentation prompt and stored response.
pub const CURRENT_PRESENTATION_VERSION: i32 = 1;

/// User-facing text and media metadata resolved from corpus and enrichment data.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PresentedTranslation {
    pub translation_id: i32,
    pub from_phrase: String,
    pub to_phrase: String,
    pub native_lang: String,
    pub foreign_lang: String,
    pub teachable: bool,
    pub image_query: Option<String>,
    pub is_phrase: bool,
    pub presentation_version: i32,
}

/// Resolves stored translation IDs into canonical card presentations.
pub struct PresentationRepository;

impl PresentationRepository {
    /// Loads the authoritative presentation for one translation.
    ///
    /// # Errors
    ///
    /// Returns an error when the database query fails.
    pub async fn load(
        pool: &PgPool,
        translation_id: i32,
    ) -> Result<Option<PresentedTranslation>, WisecrowError> {
        let presentation = sqlx::query_as::<_, PresentedTranslation>(
            "SELECT translation_id, from_phrase, to_phrase, native_lang, foreign_lang,
                    teachable, image_query, is_phrase, presentation_version
             FROM translation_presentations
             WHERE translation_id = $1",
        )
        .bind(translation_id)
        .fetch_optional(pool)
        .await?;
        Ok(presentation)
    }

    /// Loads every presentation for a language pair, ordered by translation id.
    ///
    /// # Errors
    ///
    /// Returns an error when the database query fails.
    pub async fn load_for_pair(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
    ) -> Result<Vec<PresentedTranslation>, WisecrowError> {
        let rows = sqlx::query_as::<_, PresentedTranslation>(
            "SELECT translation_id, from_phrase, to_phrase, native_lang, foreign_lang,
                    teachable, image_query, is_phrase, presentation_version
             FROM translation_presentations
             WHERE native_lang = $1 AND foreign_lang = $2
             ORDER BY translation_id",
        )
        .bind(native_lang)
        .bind(foreign_lang)
        .fetch_all(pool)
        .await?;
        Ok(rows)
    }
}
