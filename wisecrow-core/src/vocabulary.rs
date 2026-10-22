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

/// Which grammar area a ranked word deck draws from.
///
/// `All` is every admitted word except a translation a verb form owns.
/// The four classes are `word_classes.class`. A verb deck is not a class
/// filter: its cards are conjugated forms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaFilter {
    All,
    Preposition,
    Pronoun,
    Adjective,
    Adverb,
}

impl AreaFilter {
    const fn label(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Preposition => Some("preposition"),
            Self::Pronoun => Some("pronoun"),
            Self::Adjective => Some("adjective"),
            Self::Adverb => Some("adverb"),
        }
    }

    fn predicate(self) -> String {
        self.label().map_or_else(String::new, |label| {
            format!(
                "AND lower(btrim(t.to_phrase, '{trim}')) IN (
                     SELECT wc.word FROM word_classes wc
                     WHERE wc.language_id = t.to_language_id
                       AND wc.class = '{label}'
                 )",
                trim = crate::frequency::MATCH_TRIM_SQL
            )
        })
    }
}

/// Generated verb forms are not corpus rows. A reused row has
/// `owns_translation` false and is not named here.
const OWNED_VERB_FORM: &str = "AND NOT EXISTS (
             SELECT 1 FROM verb_form_translations v
             WHERE v.translation_id = t.id AND v.owns_translation
         )";

/// A cold covering-index read of a large pair measured 2.7s, and the previous
/// two-second budget cancelled the first Fast start. Eight seconds still
/// releases the connection when the plan is wrong.
const RANKED_QUERY_TIMEOUT: &str = "8s";
/// The fast page counts every admitted word, including the once-only tail, so
/// it cannot share the deck-fetch budget. Thirty seconds covers a cold scan
/// that still consults the heap, and frees the connection after that.
const WORD_COUNT_TIMEOUT: &str = "30s";
const PREPARATION_QUERY_TIMEOUT: &str = "60s";
const PREPARATION_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);
/// Ranked words the fixed preparation deck draws from.
const PREPARATION_WORDS: u32 = 10_000;
/// Ranked phrases the fixed preparation deck draws from.
const PREPARATION_PHRASES: u32 = 2_000;
/// Length of the fixed preparation deck.
const PREPARATION_DECK: usize = 10_000;

/// Which rows a ranked selection admits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionPolicy {
    /// Interactive learning: multi-character pairs as before; a
    /// single-character side only with a current teachable presentation.
    Learning,
    /// Media preparation: only words with a current teachable presentation,
    /// so nothing is generated for text that may still be renamed.
    Preparation,
}

/// What the final projection of a ranked selection returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CandidateProjection {
    /// Full learning entries.
    Learning,
    /// Translation IDs only.
    Id,
}

/// Which corpus counts a ranked statement reads.
///
/// Counts above one can use the partial covering indexes. A count of one
/// cannot, and on a large corpus that tail takes long enough to cancel the
/// deck statement. Callers run it only when the higher counts did not fill
/// the limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrequencyBand {
    AboveOne,
    One,
}

/// Tie order that picks one representative row for a normalised word: the
/// highest corpus count, then the English rendering most rows agree on, then
/// the shortest text, then the oldest row. Word promotion reuses this when it
/// links a candidate to an existing corpus row, so a card never prefers a
/// different row from the deck.
pub(crate) const REPRESENTATIVE_ORDER: &str =
    "frequency DESC NULLS LAST, agreement DESC, LENGTH(to_phrase), LENGTH(from_phrase), id";

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

const FINAL_SELECTION: &str = "FROM best
     JOIN translation_presentations p ON p.translation_id = best.id
     WHERE p.teachable
     ORDER BY best.frequency DESC, best.norm_to, best.id";

impl CandidateProjection {
    const fn columns(self) -> &'static str {
        match self {
            Self::Learning => {
                "SELECT p.translation_id, p.from_phrase, p.to_phrase, best.frequency, best.norm_to"
            }
            Self::Id => "SELECT p.translation_id, best.norm_to",
        }
    }
}

impl FrequencyBand {
    const fn sql(self) -> &'static str {
        match self {
            Self::AboveOne => "t.corpus_frequency > 1",
            Self::One => "t.corpus_frequency = 1",
        }
    }
}

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

fn ranked_statement(
    carded: IncludeCarded,
    phrase_filter: PhraseFilter,
    policy: SelectionPolicy,
    projection: CandidateProjection,
    band: FrequencyBand,
    class_sql: &str,
) -> String {
    let carded_sql = carded.predicate();
    let phrase = phrase_filter.predicate();
    let teachable = phrase_filter.teachable_predicate();
    let presentation = presentation_predicate(phrase_filter, policy);
    let frequency_sql = band.sql();
    // `$4` is the normalised forms already returned from counts above one.
    // Keeping this out of the higher-count statement means a filled deck
    // never plans the once-only tail. `>= 1` cannot use the covering indexes.
    let exclusion = match band {
        FrequencyBand::AboveOne => String::new(),
        FrequencyBand::One => format!(
            " AND lower(btrim(t.to_phrase, '{trim}')) <> ALL($4::text[])",
            trim = crate::frequency::MATCH_TRIM_SQL
        ),
    };
    let area_cut = format!("{OWNED_VERB_FORM}\n{class_sql}");
    let selected_presentation = format!("{presentation}{exclusion}\n{area_cut}");
    let scored_presentation = format!("{presentation}\n{area_cut}");
    let chosen = selected_words_cte(
        "chosen_words",
        carded_sql,
        phrase,
        &teachable,
        &selected_presentation,
        "ORDER BY max_frequency DESC, norm_to\n           LIMIT $3",
        frequency_sql,
    );
    let scored = scored_candidates_ctes(carded_sql, phrase, &scored_presentation, frequency_sql);
    format!(
        "{MEMBERSHIP_CTES},\n{chosen},\n{scored}\n{}\n{FINAL_SELECTION}",
        projection.columns()
    )
}

/// Distinct words a fast deck can draw. Same admission as
/// [`selected_words_cte`] for included cards and excluded phrases, without the
/// representative pass.
///
/// The bulk of the count is `corpus_frequency > 1`. The covering indexes admit
/// only that predicate, so a single `>= 1` scan cannot use them and heap-fetches
/// every ranked row. A frequency of one is counted separately and only adds a
/// word the bulk count does not already contain.
fn learning_word_count_statement(class_sql: &str) -> String {
    let phrase = PhraseFilter::Exclude.predicate();
    let teachable = PhraseFilter::Exclude.teachable_predicate();
    let presentation = format!(
        "{}\n{OWNED_VERB_FORM}\n{class_sql}",
        presentation_predicate(PhraseFilter::Exclude, SelectionPolicy::Learning)
    );
    let above = selected_words_cte(
        "above",
        "",
        phrase,
        &teachable,
        &presentation,
        "",
        "t.corpus_frequency > 1",
    );
    let hapax = selected_words_cte(
        "hapax_all",
        "",
        phrase,
        &teachable,
        &presentation,
        "",
        "t.corpus_frequency = 1",
    );
    format!(
        "WITH promoted_ids AS (
             SELECT DISTINCT translation_id
             FROM phrase_translations
             WHERE translation_id IS NOT NULL
         ),
         unteachable_words AS (
             SELECT word FROM word_glosses
             WHERE native_lang = $1 AND lang_code = $2 AND NOT teachable
         ),
         {above},
         {hapax}
         SELECT (SELECT count(*) FROM above) + (
             SELECT count(*) FROM hapax_all AS hapax
             WHERE NOT EXISTS (
                 SELECT 1 FROM above WHERE above.norm_to = hapax.norm_to
             )
         )"
    )
}

/// Admission of word rows by presentation state. Under `Learning`,
/// multi-character pairs are admitted as before and a single-character side
/// (Chinese `我`, Japanese `は`, English `I`) only once a current, teachable
/// presentation vouches for it, so widening candidate discovery to one
/// character exposes no unassessed noise. Under `Preparation` every word
/// needs that presentation. Phrases carry their own promotion review and
/// keep the length floor.
///
/// Applied in both ranking stages: gating only the final output would let a
/// rejected representative underfill the deck.
fn presentation_predicate(phrases: PhraseFilter, policy: SelectionPolicy) -> String {
    if matches!(phrases, PhraseFilter::Only) {
        return "AND LENGTH(t.from_phrase) >= 2 AND LENGTH(t.to_phrase) >= 2".into();
    }
    let assessed = format!(
        "EXISTS (
                 SELECT 1 FROM word_glosses g
                 WHERE g.lang_code = $2 AND g.native_lang = $1
                   AND g.word = lower(btrim(t.to_phrase, '{trim}'))
                   AND g.teachable
                   AND g.presentation_version >= {version}
               )",
        trim = crate::frequency::MATCH_TRIM_SQL,
        version = crate::presentation::CURRENT_PRESENTATION_VERSION,
    );
    match policy {
        SelectionPolicy::Learning => {
            format!("AND ((LENGTH(t.from_phrase) >= 2 AND LENGTH(t.to_phrase) >= 2) OR {assessed})")
        }
        SelectionPolicy::Preparation => format!("AND {assessed}"),
    }
}

fn selected_words_cte(
    name: &str,
    carded: &str,
    phrase: &str,
    teachable: &str,
    presentation: &str,
    order_limit: &str,
    frequency_sql: &str,
) -> String {
    format!(
        "{name} AS MATERIALIZED (
           SELECT lower(btrim(t.to_phrase, '{trim}')) AS norm_to,
                  max(t.corpus_frequency) AS max_frequency
           FROM translations t
           WHERE t.from_language_id = (SELECT id FROM languages WHERE code = $1)
             AND t.to_language_id = (SELECT id FROM languages WHERE code = $2)
             AND {frequency_sql}
             AND LENGTH(t.from_phrase) BETWEEN 1 AND 200
             AND LENGTH(t.to_phrase) BETWEEN 1 AND 200
             {carded}
             {phrase}
             {teachable}
             {presentation}
           GROUP BY lower(btrim(t.to_phrase, '{trim}'))
           {order_limit}
         )",
        trim = crate::frequency::MATCH_TRIM_SQL
    )
}

fn scored_candidates_ctes(
    carded: &str,
    phrase: &str,
    presentation: &str,
    frequency_sql: &str,
) -> String {
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
           WHERE t.from_language_id = (SELECT id FROM languages WHERE code = $1)
             AND t.to_language_id = (SELECT id FROM languages WHERE code = $2)
             AND {frequency_sql}
             AND LENGTH(t.from_phrase) BETWEEN 1 AND 200
             AND LENGTH(t.to_phrase) BETWEEN 1 AND 200
             {carded}
             {phrase}
             {presentation}
             AND lower(btrim(t.to_phrase, '{trim}')) IN (SELECT norm_to FROM chosen_words)
         ),
         best AS (
           SELECT DISTINCT ON (norm_to)
                  id, frequency, agreement, norm_to, from_phrase, to_phrase
           FROM scored
           ORDER BY norm_to, {REPRESENTATIVE_ORDER}
         )",
        trim = crate::frequency::MATCH_TRIM_SQL
    )
}

/// Statement selecting the representative corpus row for one normalised word
/// of a pair, binding `$1` native id, `$2` foreign id and `$3` the word.
/// Phrase-linked rows are excluded; carded rows are not, since promotion
/// links to the row a learner may already be reviewing.
pub(crate) fn representative_statement() -> String {
    format!(
        "SELECT id FROM (
           SELECT t.id, t.corpus_frequency AS frequency, t.from_phrase, t.to_phrase,
                  count(*) OVER (PARTITION BY lower(btrim(t.from_phrase, '{trim}'))) AS agreement
           FROM translations t
           WHERE t.from_language_id = $1 AND t.to_language_id = $2
             AND lower(btrim(t.to_phrase, '{trim}')) = $3
             AND NOT EXISTS (
               SELECT 1 FROM phrase_translations pt WHERE pt.translation_id = t.id
             )
         ) scored
         ORDER BY {REPRESENTATIVE_ORDER}
         LIMIT 1",
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
        Self::ranked_area_candidates(
            pool,
            native_lang,
            foreign_lang,
            limit,
            carded,
            phrase_filter,
            AreaFilter::All,
        )
        .await
    }

    /// Same admission as [`Self::ranked_candidates`], restricted to `area`.
    ///
    /// The class predicate is inside the statement that orders and limits.
    /// A translation owned by a verb form is absent from every area.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn ranked_area_candidates(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
        limit: u32,
        carded: IncludeCarded,
        phrase_filter: PhraseFilter,
        area: AreaFilter,
    ) -> Result<Vec<VocabularyEntry>, WisecrowError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut transaction = pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("SELECT set_config('statement_timeout', $1, true)")
            .bind(RANKED_QUERY_TIMEOUT)
            .execute(&mut *transaction)
            .await?;
        let mut request = BandRequest {
            native_lang,
            foreign_lang,
            limit,
            carded,
            phrase_filter,
            policy: SelectionPolicy::Learning,
            band: FrequencyBand::AboveOne,
            excluded: &[],
            area,
        };
        let (mut entries, norms) = fetch_learning_band(&mut transaction, &request).await?;
        if let Some(remaining) = shortfall(entries.len(), limit)? {
            // The once-only tail is a different statement, so a deck already
            // filled from counts above one never waits on it.
            sqlx::query("SELECT set_config('statement_timeout', $1, true)")
                .bind(WORD_COUNT_TIMEOUT)
                .execute(&mut *transaction)
                .await?;
            request.limit = remaining;
            request.band = FrequencyBand::One;
            request.excluded = &norms;
            let (rest, _) = fetch_learning_band(&mut transaction, &request).await?;
            entries.extend(rest);
        }
        transaction.commit().await?;
        Ok(entries)
    }

    /// Returns the fixed preparation deck for a pair: the first 10,000 ranked
    /// words with a current teachable presentation and the first 2,000 ranked
    /// phrases, interleaved to at most 10,000 IDs. A count of one is included
    /// and sorts last. The deck does not depend on how much of it a caller
    /// asks for, so a page position is stable until the ranking or
    /// presentations change.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::MediaError`] when selection exceeds sixty
    /// seconds, and any database error.
    pub async fn preparation_ids(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
    ) -> Result<Vec<i32>, WisecrowError> {
        let selection = Self::preparation_ids_unbounded(pool, native_lang, foreign_lang);
        match tokio::time::timeout(PREPARATION_DEADLINE, selection).await {
            Ok(result) => result,
            Err(_) => Err(WisecrowError::MediaError(
                "Preparation selection timed out".into(),
            )),
        }
    }

    /// How many distinct words a fast deck can draw for this pair.
    ///
    /// The count uses the same admission rules as [`Self::ranked_candidates`]
    /// for words, including a corpus count of one and excluding phrases.
    /// Unranked rows stay out. It does not build the representative row. Rows
    /// above a count of one are read from the covering index; a count of one
    /// is added only when that word is not already included.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails or the count does not fit
    /// in `u32`.
    pub async fn available_word_count(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
    ) -> Result<u32, WisecrowError> {
        Self::available_area_count(pool, native_lang, foreign_lang, AreaFilter::All).await
    }

    /// How many distinct words a fast deck can draw for this pair and area.
    ///
    /// Same admission as [`Self::ranked_area_candidates`] for included cards
    /// and excluded phrases. An owned verb form is not counted.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails or the count does not fit
    /// in `u32`.
    pub async fn available_area_count(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
        area: AreaFilter,
    ) -> Result<u32, WisecrowError> {
        let statement = learning_word_count_statement(&area.predicate());
        let mut transaction = pool.begin().await?;
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("SELECT set_config('statement_timeout', $1, true)")
            .bind(WORD_COUNT_TIMEOUT)
            .execute(&mut *transaction)
            .await?;
        // The planner's row estimate for a language is a handful, so phrase
        // exclusion becomes a nested loop over every ranked row.
        sqlx::query("SELECT set_config('enable_nestloop', 'off', true)")
            .execute(&mut *transaction)
            .await?;
        let count: i64 = sqlx::query_scalar(&statement)
            .bind(native_lang)
            .bind(foreign_lang)
            .fetch_one(&mut *transaction)
            .await?;
        transaction.commit().await?;
        u32::try_from(count)
            .map_err(|_| WisecrowError::InvalidInput("available word count exceeds bounds".into()))
    }

    async fn preparation_ids_unbounded(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
    ) -> Result<Vec<i32>, WisecrowError> {
        let mut transaction = pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *transaction)
            .await?;
        sqlx::query("SELECT set_config('statement_timeout', $1, true)")
            .bind(PREPARATION_QUERY_TIMEOUT)
            .execute(&mut *transaction)
            .await?;
        let words = preparation_page(
            &mut transaction,
            native_lang,
            foreign_lang,
            PhraseFilter::Exclude,
            PREPARATION_WORDS,
        )
        .await?;
        let phrases = preparation_page(
            &mut transaction,
            native_lang,
            foreign_lang,
            PhraseFilter::Only,
            PREPARATION_PHRASES,
        )
        .await?;
        transaction.commit().await?;
        Ok(interleave_deck(words, phrases, PREPARATION_DECK))
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

    /// Conjugated forms for a pair, most frequent lemma first, then slot order.
    ///
    /// `slot_order` is that language's current table. A stored slot the table
    /// no longer names stays available and sorts after the named slots.
    /// Frequency is the highest corpus count above one among the lemma's
    /// classified forms, and only for a lemma that has a stored form. Each
    /// of those words is one lookup on the ranked-word index. A count of one
    /// sorts with an unranked lemma. The card face is the stored native face:
    /// `translation_presentations` is defined from the whole translations
    /// table, so joining it reads the corpus and the session times out.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails. A plan that does not
    /// finish within the ranked-word budget does too, so a bad plan cannot
    /// leave the learner on a loading screen.
    pub async fn verb_form_candidates(
        pool: &PgPool,
        native_lang: &str,
        foreign_lang: &str,
        limit: u32,
        carded: IncludeCarded,
        slot_order: &[&str],
    ) -> Result<Vec<VocabularyEntry>, WisecrowError> {
        if limit == 0 || slot_order.is_empty() {
            return Ok(Vec::new());
        }
        let carded_sql = match carded {
            IncludeCarded::Yes => "",
            IncludeCarded::No => {
                "AND forms.translation_id NOT IN (SELECT translation_id FROM carded_ids)"
            }
        };
        let trim = crate::frequency::MATCH_TRIM_SQL;
        let statement = format!(
            "WITH carded_ids AS MATERIALIZED (
                 SELECT DISTINCT translation_id FROM cards WHERE translation_id IS NOT NULL
             ),
             forms AS MATERIALIZED (
                 SELECT t.id AS translation_id,
                        vft.native_face AS from_phrase,
                        t.to_phrase,
                        vf.lemma,
                        vf.slot,
                        vf.id AS form_id
                 FROM verb_form_translations vft
                 JOIN verb_forms vf ON vf.id = vft.verb_form_id
                 JOIN languages nl ON nl.id = vft.native_language_id
                 JOIN languages fl ON fl.id = vf.language_id
                 JOIN translations t ON t.id = vft.translation_id
                 LEFT JOIN owned_presentations op ON op.translation_id = t.id
                 WHERE nl.code = $1 AND fl.code = $2
                   AND COALESCE(op.teachable, TRUE)
             ),
             verb_words AS MATERIALIZED (
                 SELECT wc.lemma, wc.word
                 FROM word_classes wc
                 WHERE wc.language_id = (SELECT id FROM languages WHERE code = $2)
                   AND wc.class = 'verb'
                   AND wc.lemma IN (SELECT lemma FROM forms)
             ),
             lemma_freq AS MATERIALIZED (
                 SELECT vw.lemma, max(hit.freq) AS freq
                 FROM verb_words vw
                 JOIN LATERAL (
                     SELECT t.corpus_frequency AS freq
                     FROM translations t
                     WHERE t.from_language_id = (SELECT id FROM languages WHERE code = $1)
                       AND t.to_language_id = (SELECT id FROM languages WHERE code = $2)
                       AND lower(btrim(t.to_phrase, '{trim}')) = vw.word
                       AND t.corpus_frequency > 1
                       AND LENGTH(t.from_phrase) BETWEEN 1 AND 200
                       AND LENGTH(t.to_phrase) BETWEEN 1 AND 200
                       AND NOT EXISTS (
                           SELECT 1 FROM word_promotions owned
                           WHERE owned.translation_id = t.id AND owned.owns_translation
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM phrase_translations owned
                           WHERE owned.translation_id = t.id AND owned.owns_translation
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM verb_form_translations owned
                           WHERE owned.translation_id = t.id AND owned.owns_translation
                       )
                       AND NOT EXISTS (
                           SELECT 1 FROM reader_translations owned
                           WHERE owned.translation_id = t.id AND owned.owns_translation
                       )
                     ORDER BY t.corpus_frequency DESC
                     LIMIT 1
                 ) hit ON TRUE
                 GROUP BY vw.lemma
             )
             SELECT forms.translation_id, forms.from_phrase, forms.to_phrase,
                    COALESCE(lf.freq, 0)
             FROM forms
             LEFT JOIN lemma_freq lf ON lf.lemma = forms.lemma
             WHERE TRUE
             {carded_sql}
             ORDER BY lf.freq DESC NULLS LAST, forms.lemma,
                      array_position($4::text[], forms.slot), forms.form_id
             LIMIT $3"
        );
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
            .bind(slot_order)
            .fetch_all(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(rows
            .into_iter()
            .map(
                |(translation_id, from_phrase, to_phrase, frequency)| VocabularyEntry {
                    translation_id,
                    from_phrase,
                    to_phrase,
                    frequency,
                },
            )
            .collect())
    }
}

struct BandRequest<'a> {
    native_lang: &'a str,
    foreign_lang: &'a str,
    limit: u32,
    carded: IncludeCarded,
    phrase_filter: PhraseFilter,
    policy: SelectionPolicy,
    band: FrequencyBand,
    excluded: &'a [String],
    area: AreaFilter,
}

fn shortfall(fetched: usize, limit: u32) -> Result<Option<u32>, WisecrowError> {
    let fetched = u32::try_from(fetched)
        .map_err(|_| WisecrowError::InvalidInput("ranked page exceeds bounds".into()))?;
    Ok(if fetched >= limit {
        None
    } else {
        Some(limit - fetched)
    })
}

async fn preparation_page(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    native_lang: &str,
    foreign_lang: &str,
    phrase_filter: PhraseFilter,
    limit: u32,
) -> Result<Vec<i32>, WisecrowError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut request = BandRequest {
        native_lang,
        foreign_lang,
        limit,
        carded: IncludeCarded::Yes,
        phrase_filter,
        policy: SelectionPolicy::Preparation,
        band: FrequencyBand::AboveOne,
        excluded: &[],
        area: AreaFilter::All,
    };
    let (mut ids, norms) = fetch_id_band(transaction, &request).await?;
    if let Some(remaining) = shortfall(ids.len(), limit)? {
        request.limit = remaining;
        request.band = FrequencyBand::One;
        request.excluded = &norms;
        let (rest, _) = fetch_id_band(transaction, &request).await?;
        ids.extend(rest);
    }
    Ok(ids)
}

async fn fetch_learning_band(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request: &BandRequest<'_>,
) -> Result<(Vec<VocabularyEntry>, Vec<String>), WisecrowError> {
    let statement = ranked_statement(
        request.carded,
        request.phrase_filter,
        request.policy,
        CandidateProjection::Learning,
        request.band,
        &request.area.predicate(),
    );
    let rows = match request.band {
        FrequencyBand::AboveOne => {
            sqlx::query_as::<_, (i32, String, String, i32, String)>(&statement)
                .bind(request.native_lang)
                .bind(request.foreign_lang)
                .bind(i64::from(request.limit))
                .fetch_all(&mut **transaction)
                .await?
        }
        FrequencyBand::One => {
            sqlx::query_as::<_, (i32, String, String, i32, String)>(&statement)
                .bind(request.native_lang)
                .bind(request.foreign_lang)
                .bind(i64::from(request.limit))
                .bind(request.excluded)
                .fetch_all(&mut **transaction)
                .await?
        }
    };
    let mut entries = Vec::with_capacity(rows.len());
    let mut norms = Vec::with_capacity(rows.len());
    for (id, from_phrase, to_phrase, frequency, norm) in rows {
        entries.push(VocabularyEntry {
            translation_id: id,
            from_phrase,
            to_phrase,
            frequency,
        });
        norms.push(norm);
    }
    Ok((entries, norms))
}

async fn fetch_id_band(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request: &BandRequest<'_>,
) -> Result<(Vec<i32>, Vec<String>), WisecrowError> {
    let statement = ranked_statement(
        request.carded,
        request.phrase_filter,
        request.policy,
        CandidateProjection::Id,
        request.band,
        &request.area.predicate(),
    );
    let rows = match request.band {
        FrequencyBand::AboveOne => {
            sqlx::query_as::<_, (i32, String)>(&statement)
                .bind(request.native_lang)
                .bind(request.foreign_lang)
                .bind(i64::from(request.limit))
                .fetch_all(&mut **transaction)
                .await?
        }
        FrequencyBand::One => {
            sqlx::query_as::<_, (i32, String)>(&statement)
                .bind(request.native_lang)
                .bind(request.foreign_lang)
                .bind(i64::from(request.limit))
                .bind(request.excluded)
                .fetch_all(&mut **transaction)
                .await?
        }
    };
    let mut ids = Vec::with_capacity(rows.len());
    let mut norms = Vec::with_capacity(rows.len());
    for (id, norm) in rows {
        ids.push(id);
        norms.push(norm);
    }
    Ok((ids, norms))
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
    fn ranked_deck_keeps_frequency_one_off_the_covering_index_scan() {
        let frequent = ranked_statement(
            IncludeCarded::Yes,
            PhraseFilter::Exclude,
            SelectionPolicy::Learning,
            CandidateProjection::Learning,
            FrequencyBand::AboveOne,
            "",
        );
        assert!(frequent.contains("t.corpus_frequency > 1"));
        assert!(!frequent.contains("t.corpus_frequency = 1"));
        assert!(!frequent.contains("corpus_frequency >= 1"));
        assert!(!frequent.contains("$4"));

        let once = ranked_statement(
            IncludeCarded::Yes,
            PhraseFilter::Exclude,
            SelectionPolicy::Learning,
            CandidateProjection::Learning,
            FrequencyBand::One,
            "",
        );
        assert!(once.contains("t.corpus_frequency = 1"));
        assert!(!once.contains("t.corpus_frequency > 1"));
        assert!(once.contains("<> ALL($4::text[])"));
        assert!(!once.contains("corpus_frequency >= 1"));
    }

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
