use crate::errors::WisecrowError;
use sqlx::PgPool;

#[derive(Debug, Clone)]
pub struct VocabularyEntry {
    pub translation_id: i32,
    pub from_phrase: String,
    pub to_phrase: String,
    pub frequency: i32,
}

pub struct VocabularyQuery;

impl VocabularyQuery {
    /// Returns translations that don't yet have associated SRS cards, ordered
    /// by frequency descending.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn unlearned(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
        limit: u32,
    ) -> Result<Vec<VocabularyEntry>, WisecrowError> {
        // Translations that no user has yet started a card on are universally
        // unlearned. Per-user "what should I learn next" should additionally
        // filter against this user's cards once consumed.
        let rows = sqlx::query_as::<_, (i32, String, String, i32)>(
            "SELECT id, from_phrase, to_phrase, frequency FROM (
               SELECT DISTINCT ON (t.from_phrase)
                      t.id, t.from_phrase, t.to_phrase, t.frequency
               FROM translations t
               JOIN languages fl ON t.from_language_id = fl.id
               JOIN languages tl ON t.to_language_id = tl.id
               LEFT JOIN cards c ON c.translation_id = t.id
               WHERE fl.code = $1 AND tl.code = $2 AND c.id IS NULL
                 AND t.frequency > 1
                 AND LENGTH(t.from_phrase) BETWEEN 2 AND 200
                 AND LENGTH(t.to_phrase) BETWEEN 2 AND 200
               ORDER BY t.from_phrase, t.frequency DESC
             ) best
             ORDER BY best.frequency DESC
             LIMIT $3",
        )
        .bind(native_lang)
        .bind(foreign_lang)
        .bind(i64::from(limit))
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, from, to, freq)| VocabularyEntry {
                translation_id: id,
                from_phrase: from,
                to_phrase: to,
                frequency: freq,
            })
            .collect())
    }

    /// Returns translations whose card (for the given user) is in any of the
    /// given FSRS states, optionally requiring a minimum stability. Ordered by
    /// frequency descending.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn learned(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
        user_id: i32,
        seed_states: &[i16],
        min_stability: Option<f32>,
        limit: u32,
    ) -> Result<Vec<VocabularyEntry>, WisecrowError> {
        let rows = sqlx::query_as::<_, (i32, String, String, i32)>(
            "SELECT t.id, t.from_phrase, t.to_phrase, t.frequency
             FROM translations t
             JOIN languages fl ON t.from_language_id = fl.id
             JOIN languages tl ON t.to_language_id = tl.id
             JOIN cards c ON c.translation_id = t.id
             WHERE fl.code = $1 AND tl.code = $2
               AND c.user_id = $3
               AND c.state = ANY($4)
               AND ($5::REAL IS NULL OR c.stability >= $5)
             ORDER BY t.frequency DESC
             LIMIT $6",
        )
        .bind(native_lang)
        .bind(foreign_lang)
        .bind(user_id)
        .bind(seed_states)
        .bind(min_stability)
        .bind(i64::from(limit))
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, from, to, freq)| VocabularyEntry {
                translation_id: id,
                from_phrase: from,
                to_phrase: to,
                frequency: freq,
            })
            .collect())
    }
}
