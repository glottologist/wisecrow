use crate::errors::WisecrowError;
use sqlx::PgPool;

#[derive(Debug, Clone)]
pub struct VocabularyEntry {
    pub translation_id: i32,
    pub from_phrase: String,
    pub to_phrase: String,
    pub frequency: i32,
}

/// Whether rows that already have an SRS card stay in the result. SRS
/// seeding excludes them; the passive fast mode, which owns no cards,
/// includes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IncludeCarded {
    Yes,
    No,
}

/// Membership test against `phrase_translations.translation_id`, separating
/// promoted phrases from ordinary word rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhraseFilter {
    Only,
    Exclude,
}

const RANKED_QUERY_TIMEOUT: &str = "2s";

const MEMBERSHIP_CTES: &str = "WITH carded_ids AS MATERIALIZED (
         SELECT DISTINCT translation_id FROM cards WHERE translation_id IS NOT NULL
     ),
     promoted_ids AS MATERIALIZED (
         SELECT DISTINCT translation_id
         FROM phrase_translations
         WHERE translation_id IS NOT NULL
     ),
     unteachable_words AS MATERIALIZED (
         SELECT word FROM word_glosses
         WHERE native_lang = $1 AND lang_code = $2 AND NOT teachable
     )";

const FINAL_SELECTION: &str = "SELECT p.translation_id, p.from_phrase, p.to_phrase, best.frequency
     FROM best
     JOIN translation_presentations p ON p.translation_id = best.id
     WHERE p.teachable
     ORDER BY best.frequency DESC, best.norm_to, best.id";

impl IncludeCarded {
    const fn predicate(self) -> &'static str {
        match self {
            Self::Yes => "",
            Self::No => "AND t.id NOT IN (SELECT translation_id FROM carded_ids)",
        }
    }
}

impl PhraseFilter {
    const fn predicate(self) -> &'static str {
        match self {
            Self::Only => "AND t.id IN (SELECT translation_id FROM promoted_ids)",
            Self::Exclude => "AND t.id NOT IN (SELECT translation_id FROM promoted_ids)",
        }
    }

    fn teachable_predicate(self) -> String {
        match self {
            Self::Only => String::new(),
            Self::Exclude => format!(
                "AND lower(btrim(t.to_phrase, '{}')) NOT IN
                     (SELECT word FROM unteachable_words)",
                crate::frequency::MATCH_TRIM_SQL
            ),
        }
    }
}

pub struct VocabularyQuery;

fn ranked_statement(carded: IncludeCarded, phrase_filter: PhraseFilter) -> String {
    let carded = carded.predicate();
    let phrase = phrase_filter.predicate();
    let teachable = phrase_filter.teachable_predicate();
    let selected = selected_words_cte(carded, phrase, &teachable);
    let scored = scored_candidates_ctes(carded, phrase);
    format!("{MEMBERSHIP_CTES},\n{selected},\n{scored}\n{FINAL_SELECTION}")
}

fn selected_words_cte(carded: &str, phrase: &str, teachable: &str) -> String {
    format!(
        "selected_words AS MATERIALIZED (
           SELECT lower(btrim(t.to_phrase, '{trim}')) AS norm_to,
                  max(t.corpus_frequency) AS max_frequency
           FROM translations t
           JOIN languages fl ON fl.id = t.from_language_id
           JOIN languages tl ON tl.id = t.to_language_id
           WHERE fl.code = $1 AND tl.code = $2
             AND t.corpus_frequency > 1
             AND LENGTH(t.from_phrase) BETWEEN 2 AND 200
             AND LENGTH(t.to_phrase) BETWEEN 2 AND 200
             {carded}
             {phrase}
             {teachable}
           GROUP BY lower(btrim(t.to_phrase, '{trim}'))
           ORDER BY max_frequency DESC, norm_to
           LIMIT $3
         )",
        trim = crate::frequency::MATCH_TRIM_SQL
    )
}

fn scored_candidates_ctes(carded: &str, phrase: &str) -> String {
    format!(
        "scored AS (
           SELECT t.id, t.from_phrase, t.to_phrase,
                  t.corpus_frequency AS frequency,
                  lower(btrim(t.to_phrase, '{trim}')) AS norm_to,
                  count(*) OVER (
                    PARTITION BY lower(btrim(t.to_phrase, '{trim}')),
                                 lower(btrim(t.from_phrase, '{trim}'))
                  ) AS agreement
           FROM translations t
           JOIN languages fl ON fl.id = t.from_language_id
           JOIN languages tl ON tl.id = t.to_language_id
           WHERE fl.code = $1 AND tl.code = $2
             AND t.corpus_frequency > 1
             AND LENGTH(t.from_phrase) BETWEEN 2 AND 200
             AND LENGTH(t.to_phrase) BETWEEN 2 AND 200
             {carded}
             {phrase}
             AND lower(btrim(t.to_phrase, '{trim}')) = ANY (
               ARRAY(SELECT norm_to FROM selected_words)
             )
         ),
         best AS (
           SELECT DISTINCT ON (norm_to)
                  id, frequency, agreement, norm_to, from_phrase, to_phrase
           FROM scored
           ORDER BY norm_to, frequency DESC, agreement DESC,
                    LENGTH(to_phrase), LENGTH(from_phrase), id
         )",
        trim = crate::frequency::MATCH_TRIM_SQL
    )
}

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
        Self::ranked_candidates(
            pool,
            native_lang,
            foreign_lang,
            limit,
            IncludeCarded::No,
            PhraseFilter::Exclude,
        )
        .await
    }

    /// Selects distinct ranked forms before scoring their representative rows.
    /// Canonical presentations override corpus display text and rejected forms
    /// are omitted.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn ranked_candidates(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
        limit: u32,
        carded: IncludeCarded,
        phrase_filter: PhraseFilter,
    ) -> Result<Vec<VocabularyEntry>, WisecrowError> {
        let statement = ranked_statement(carded, phrase_filter);
        let mut transaction = pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("SELECT set_config('statement_timeout', $1, true)")
            .bind(RANKED_QUERY_TIMEOUT)
            .execute(&mut *transaction)
            .await?;
        let rows = sqlx::query_as::<_, (i32, String, String, i32)>(&statement)
            .bind(native_lang)
            .bind(foreign_lang)
            .bind(i64::from(limit))
            .fetch_all(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(rows
            .into_iter()
            .map(|(id, from, to, frequency)| VocabularyEntry {
                translation_id: id,
                from_phrase: from,
                to_phrase: to,
                frequency,
            })
            .collect())
    }

    /// Returns a user's cards in selected FSRS states, highest frequency first.
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
            "SELECT t.id, t.from_phrase, t.to_phrase, COALESCE(t.corpus_frequency, 0)
             FROM translations t
             JOIN languages fl ON t.from_language_id = fl.id
             JOIN languages tl ON t.to_language_id = tl.id
             JOIN cards c ON c.translation_id = t.id
             WHERE fl.code = $1 AND tl.code = $2
               AND c.user_id = $3
               AND c.state = ANY($4)
               AND ($5::REAL IS NULL OR c.stability >= $5)
             ORDER BY t.corpus_frequency DESC NULLS LAST
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

/// Interleaves ranked words and phrases into one deck of at most `size`:
/// every 5th slot (positions 4, 9, 14, …) takes the next phrase while any
/// remain, all other slots take the next word; when either list runs dry the
/// other fills the remainder, so the deck reaches
/// `min(size, words.len() + phrases.len())`. Callers cap the phrase pool at
/// `size / 5` when fetching, which is what holds the 80/20 shape.
#[must_use]
pub fn interleave_deck<T>(words: Vec<T>, phrases: Vec<T>, size: usize) -> Vec<T> {
    let mut words = words.into_iter();
    let mut phrases = phrases.into_iter();
    let mut deck = Vec::with_capacity(size);
    while deck.len() < size {
        let phrase_slot = deck.len() % 5 == 4;
        let next = if phrase_slot {
            phrases.next().or_else(|| words.next())
        } else {
            words.next().or_else(|| phrases.next())
        };
        match next {
            Some(item) => deck.push(item),
            None => break,
        }
    }
    deck
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interleave_ten_items_two_phrases() {
        let words: Vec<&str> = vec!["w0", "w1", "w2", "w3", "w4", "w5", "w6", "w7"];
        let phrases = vec!["p0", "p1"];
        assert_eq!(
            interleave_deck(words, phrases, 10),
            vec!["w0", "w1", "w2", "w3", "p0", "w4", "w5", "w6", "w7", "p1"]
        );
    }

    #[test]
    fn interleave_no_phrases_fills_with_words() {
        let words: Vec<usize> = (0..100).collect();
        let deck = interleave_deck(words, Vec::new(), 100);
        assert_eq!(deck, (0..100).collect::<Vec<_>>());
    }

    #[test]
    fn interleave_no_words_fills_with_phrases() {
        let phrases: Vec<usize> = (0..10).collect();
        // With no words, phrases fill every slot: the deck reaches
        // min(size, words + phrases), and the 1-in-5 shape only holds while
        // both pools last.
        assert_eq!(
            interleave_deck(Vec::new(), phrases, 10),
            (0..10).collect::<Vec<_>>()
        );
    }

    #[test]
    fn interleave_size_not_multiple_of_five() {
        let words: Vec<&str> = vec!["w0", "w1", "w2", "w3", "w4", "w5"];
        let phrases = vec!["p0", "p1"];
        let deck = interleave_deck(words, phrases, 7);
        assert_eq!(deck.len(), 7);
        assert_eq!(deck.iter().filter(|i| i.starts_with('p')).count(), 1);
    }

    #[test]
    fn interleave_size_zero_is_empty() {
        assert!(interleave_deck::<u8>(vec![1, 2], vec![3], 0).is_empty());
    }

    #[test]
    fn interleave_short_both_pools() {
        let deck = interleave_deck(vec!["w0", "w1", "w2"], vec!["p0"], 100);
        assert_eq!(deck.len(), 4);
    }
}
