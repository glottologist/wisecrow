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
    /// Returns translations that don't yet have associated SRS cards, one per
    /// distinct word of the language being learned, ordered by frequency
    /// descending.
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
        //
        // The deck is deduplicated on the phrase being *learned*, normalised.
        // Deduplicating on `from_phrase` instead put one foreign word into as
        // many cards as it had native-language partners: a Gaelic deck opened
        // with ten separate cards for "Tha", differing only in the English
        // beside them, because every row carrying that word shares the one
        // corpus count and so ties. Normalising matters as much as the side
        // does — "Tha", "Tha." and "Tha?" are three strings and one word.
        //
        // Which partner to show is then chosen by consensus. Corpus counts say
        // nothing about it, and the alternative — picking arbitrarily among
        // rows that all tie — put "AZ has" and "thatthe" against "Tha" while
        // "Yes" sat in the same corpus. `agreement` counts how many of a word's
        // stored pairs give the same native phrase, so a translation the corpus
        // repeats beats a one-off, and mojibake and run-together fragments lose
        // by construction rather than by a rule enumerating them.
        let statement = format!(
            "SELECT id, from_phrase, to_phrase, frequency FROM (
               SELECT DISTINCT ON (norm_to) id, from_phrase, to_phrase, frequency
               FROM (
                 SELECT t.id, t.from_phrase, t.to_phrase, t.frequency,
                        lower(btrim(t.to_phrase, '{trim}')) AS norm_to,
                        count(*) OVER (
                          PARTITION BY lower(btrim(t.to_phrase, '{trim}')),
                                       lower(btrim(t.from_phrase, '{trim}'))
                        ) AS agreement
                 FROM translations t
                 JOIN languages fl ON t.from_language_id = fl.id
                 JOIN languages tl ON t.to_language_id = tl.id
                 LEFT JOIN cards c ON c.translation_id = t.id
                 WHERE fl.code = $1 AND tl.code = $2 AND c.id IS NULL
                   AND t.frequency > 1
                   AND LENGTH(t.from_phrase) BETWEEN 2 AND 200
                   AND LENGTH(t.to_phrase) BETWEEN 2 AND 200
               ) scored
               ORDER BY norm_to,
                        frequency DESC,
                        agreement DESC,
                        LENGTH(to_phrase),
                        LENGTH(from_phrase),
                        id
             ) best
             ORDER BY best.frequency DESC
             LIMIT $3",
            trim = crate::frequency::MATCH_TRIM_SQL
        );
        let rows = sqlx::query_as::<_, (i32, String, String, i32)>(&statement)
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
