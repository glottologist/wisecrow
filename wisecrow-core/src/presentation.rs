use sqlx::PgPool;

use crate::errors::WisecrowError;

/// Most IDs one [`PresentationRepository::load_selected`] call accepts.
pub const SELECTED_BATCH: usize = 100;

/// Version of the canonical word-presentation prompt and stored response.
/// Version 2 requires `display_form` to keep the requested word's normalised
/// spelling and both text fields to be meaningful text.
pub const CURRENT_PRESENTATION_VERSION: i32 = 2;

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

    /// Loads at most 100 exact IDs for one pair, preserving input order.
    ///
    /// # Errors
    ///
    /// Rejects duplicate, missing or out-of-pair IDs and batches over 100.
    pub async fn load_selected(
        pool: &PgPool,
        ids: &[i32],
        native_lang: &str,
        foreign_lang: &str,
    ) -> Result<Vec<PresentedTranslation>, WisecrowError> {
        let distinct: std::collections::HashSet<i32> = ids.iter().copied().collect();
        if ids.len() > SELECTED_BATCH || distinct.len() != ids.len() {
            return Err(WisecrowError::InvalidInput(
                "Invalid presentation ID batch".into(),
            ));
        }
        let mut transaction = pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *transaction)
            .await?;
        let rows = sqlx::query_as::<_, PresentedTranslation>(
            "SELECT p.translation_id, p.from_phrase, p.to_phrase, p.native_lang, p.foreign_lang,
                    p.teachable, p.image_query, p.is_phrase, p.presentation_version
             FROM unnest($1::INTEGER[]) WITH ORDINALITY AS requested(id, position)
             JOIN translation_presentations p ON p.translation_id = requested.id
             WHERE p.native_lang = $2 AND p.foreign_lang = $3
             ORDER BY requested.position",
        )
        .bind(ids)
        .bind(native_lang)
        .bind(foreign_lang)
        .fetch_all(&mut *transaction)
        .await?;
        if rows.len() != ids.len() {
            return Err(WisecrowError::InvalidInput(
                "Selected presentation missing or belongs to another pair".into(),
            ));
        }
        transaction.commit().await?;
        Ok(rows)
    }
}
