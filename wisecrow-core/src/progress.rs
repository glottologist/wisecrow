//! Per-learner progress, rolled up for the dashboard.
//!
//! Nothing here is stored. Every figure is read live from records the learning
//! paths already keep (card state, the review log, grammar mastery, placement
//! runs and n-back sessions) for one learner at a time. Should the page ever
//! grow slow, a materialised daily summary is the obvious follow-up, and
//! nothing here prevents it.
//!
//! A card's language pair is read from the translation it teaches, as
//! `translation_presentations` defines it: the source language is the native
//! one and the target the foreign one.

use chrono::{DateTime, Days, NaiveDate, NaiveTime, Utc};
use sqlx::PgPool;
use wisecrow_learning::mastery::{band_from, Band};

use crate::errors::WisecrowError;
use crate::grammar::mastery::{MasteryRepository, MasteryRow};
use crate::grammar::placement::{self, CompletedPlacement};
use crate::srs::scheduler::CardStatus;

/// A foreign language the learner has touched, with the native languages
/// their vocabulary was practised from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageScope {
    pub foreign: String,
    pub foreign_name: String,
    /// Sorted. Empty where only grammar, placement or n-back touched the
    /// language.
    pub natives: Vec<String>,
}

/// Days the dashboard's review strip covers.
pub const REVIEW_DAYS: u32 = 30;

/// How many of a learner's cards for one pair sit in each FSRS state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VocabularyCounts {
    pub new: i64,
    pub learning: i64,
    pub review: i64,
    pub relearning: i64,
    /// Cards due before the end of the current UTC day.
    pub due_today: i64,
}

/// Reviews made on one UTC day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DailyCount {
    pub day: NaiveDate,
    pub count: i64,
}

/// How many grammar points of one CEFR level sit in each brainmap band.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LevelBands {
    pub level: String,
    pub unseen: u32,
    pub red: u32,
    pub amber: u32,
    pub green: u32,
    pub provisional_red: u32,
    pub provisional_amber: u32,
    pub provisional_green: u32,
}

impl LevelBands {
    fn count(&mut self, band: Band) {
        let tally = match band {
            Band::Unseen => &mut self.unseen,
            Band::Red => &mut self.red,
            Band::Amber => &mut self.amber,
            Band::Green => &mut self.green,
            Band::ProvisionalRed => &mut self.provisional_red,
            Band::ProvisionalAmber => &mut self.provisional_amber,
            Band::ProvisionalGreen => &mut self.provisional_green,
        };
        *tally = tally.saturating_add(1);
    }
}

/// The learner's dual n-back record from one native language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NbackSummary {
    pub native: String,
    /// The highest n reached in any counted session: a ceiling the learner
    /// touched, not a level they have mastered.
    pub highest_n: i16,
    pub sessions: i64,
}

/// One native language's vocabulary in a foreign language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairVocabulary {
    pub native: String,
    pub counts: VocabularyCounts,
    pub reviews: Vec<DailyCount>,
    pub verb_forms: wisecrow_dto::VerbFormProgressDto,
}

/// Everything the dashboard shows for one foreign language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageProgress {
    pub foreign: String,
    pub foreign_name: String,
    pub vocabulary: Vec<PairVocabulary>,
    pub grammar: Vec<LevelBands>,
    pub placement: Option<CompletedPlacement>,
    pub nback: Vec<NbackSummary>,
    pub books: wisecrow_dto::BookProgressDto,
}

/// Reads one learner's progress.
pub struct ProgressQuery;

impl ProgressQuery {
    /// Every foreign language the learner has made progress in, by name.
    ///
    /// The languages are derived from the progress itself. Grammar rows carry
    /// no native language and n-back is grouped separately, so neither adds a
    /// native language here; an n-back session counts only once it has
    /// finished with an answered trial, the rule the n-back summary applies.
    ///
    /// # Errors
    ///
    /// Returns an error when the query fails.
    pub async fn languages_for_user(
        pool: &PgPool,
        user_id: i32,
    ) -> Result<Vec<LanguageScope>, WisecrowError> {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT foreign_lang, native_lang FROM sessions WHERE user_id = $1
             UNION
             SELECT tl.code, fl.code
             FROM cards c
             JOIN translations t ON t.id = c.translation_id
             JOIN languages fl ON fl.id = t.from_language_id
             JOIN languages tl ON tl.id = t.to_language_id
             WHERE c.user_id = $1
             UNION
             SELECT foreign_lang, NULL FROM dnb_sessions
             WHERE user_id = $1 AND completed_at IS NOT NULL AND trials_completed > 0
             UNION
             SELECT l.code, NULL
             FROM grammar_sessions gs JOIN languages l ON l.id = gs.language_id
             WHERE gs.user_id = $1
             UNION
             SELECT l.code, NULL
             FROM grammar_mastery gm
             JOIN grammar_rules gr ON gr.id = gm.rule_id
             JOIN languages l ON l.id = gr.language_id
             WHERE gm.user_id = $1
             UNION
             SELECT l.code, NULL
             FROM placement_attempts pa JOIN languages l ON l.id = pa.language_id
             WHERE pa.user_id = $1
             UNION
             SELECT fl.code, NULL
             FROM reading_positions rp
             JOIN books b ON b.file = rp.file AND b.source_language_id = rp.language_id
             JOIN book_sentences s ON s.book_id = b.id
             JOIN book_translations t ON t.sentence_id = s.id
             JOIN languages fl ON fl.id = t.language_id
             WHERE rp.user_id = $1
             ORDER BY 1, 2 NULLS FIRST",
        )
        .bind(user_id)
        .fetch_all(pool)
        .await?;
        Ok(group_languages(rows))
    }

    /// Counts the learner's cards for one pair by FSRS state, with those due
    /// before the end of the UTC day.
    ///
    /// # Errors
    ///
    /// Returns an error when the query fails.
    pub async fn vocabulary_counts(
        pool: &PgPool,
        user_id: i32,
        native: &str,
        foreign: &str,
    ) -> Result<VocabularyCounts, WisecrowError> {
        let (new, learning, review, relearning, due_today): (i64, i64, i64, i64, i64) =
            sqlx::query_as(
                "SELECT COUNT(*) FILTER (WHERE c.state = $4),
                        COUNT(*) FILTER (WHERE c.state = $5),
                        COUNT(*) FILTER (WHERE c.state = $6),
                        COUNT(*) FILTER (WHERE c.state = $7),
                        COUNT(*) FILTER (
                            WHERE c.due < (date_trunc('day', NOW() AT TIME ZONE 'UTC')
                                           + INTERVAL '1 day') AT TIME ZONE 'UTC')
                 FROM cards c
                 JOIN translations t ON t.id = c.translation_id
                 JOIN languages fl ON fl.id = t.from_language_id
                 JOIN languages tl ON tl.id = t.to_language_id
                 WHERE c.user_id = $1 AND fl.code = $2 AND tl.code = $3",
            )
            .bind(user_id)
            .bind(native)
            .bind(foreign)
            .bind(CardStatus::New.to_db())
            .bind(CardStatus::Learning.to_db())
            .bind(CardStatus::Review.to_db())
            .bind(CardStatus::Relearning.to_db())
            .fetch_one(pool)
            .await?;
        Ok(VocabularyCounts {
            new,
            learning,
            review,
            relearning,
            due_today,
        })
    }

    /// Reviews of one pair per UTC day, oldest first, ending today.
    ///
    /// Days without reviews are present with a count of zero, so the strip
    /// always has `days` entries. Every review source counts, n-back included,
    /// because each one moved a card.
    ///
    /// # Errors
    ///
    /// Returns an error when the query fails.
    pub async fn daily_reviews(
        pool: &PgPool,
        user_id: i32,
        native: &str,
        foreign: &str,
        days: u32,
    ) -> Result<Vec<DailyCount>, WisecrowError> {
        let today = Utc::now().date_naive();
        let first_day = today
            .checked_sub_days(Days::new(u64::from(days.saturating_sub(1))))
            .unwrap_or(NaiveDate::MIN);
        let since: DateTime<Utc> = first_day.and_time(NaiveTime::MIN).and_utc();
        let counted: Vec<(NaiveDate, i64)> = sqlx::query_as(
            "SELECT (re.occurred_at AT TIME ZONE 'UTC')::date AS day, COUNT(*)
             FROM review_events re
             JOIN translations t ON t.id = re.translation_id
             JOIN languages fl ON fl.id = t.from_language_id
             JOIN languages tl ON tl.id = t.to_language_id
             WHERE re.user_id = $1 AND fl.code = $2 AND tl.code = $3
               AND re.occurred_at >= $4
             GROUP BY day
             ORDER BY day",
        )
        .bind(user_id)
        .bind(native)
        .bind(foreign)
        .bind(since)
        .fetch_all(pool)
        .await?;
        Ok(zero_fill(today, days, &counted))
    }

    /// Counts a language's grammar points per CEFR level and band.
    ///
    /// Every syllabus point is counted, attempted or not, so the grid shows
    /// what remains as well as what is known; the bands are the brainmap's own.
    ///
    /// # Errors
    ///
    /// Returns an error when the query fails.
    pub async fn grammar_bands(
        pool: &PgPool,
        user_id: i32,
        foreign: &str,
    ) -> Result<Vec<LevelBands>, WisecrowError> {
        let rows = MasteryRepository::mastery_for_language(pool, user_id, foreign).await?;
        Ok(count_bands(&rows))
    }

    /// The learner's most recent finished placement run for a language.
    ///
    /// A run that passed no level is still reported, with no level reached,
    /// so that it reads differently from never having taken the test.
    ///
    /// # Errors
    ///
    /// Returns an error when a query fails.
    pub async fn placement_summary(
        pool: &PgPool,
        user_id: i32,
        foreign: &str,
    ) -> Result<Option<CompletedPlacement>, WisecrowError> {
        placement::latest_completed(pool, user_id, foreign).await
    }

    /// The learner's dual n-back record for a language, per native language.
    ///
    /// A session counts once it has finished with at least one answered trial.
    /// Its peak is set to the opening level when the session is created, so an
    /// abandoned or unanswered session would otherwise report a level the
    /// learner never played at.
    ///
    /// # Errors
    ///
    /// Returns an error when the query fails.
    pub async fn nback_summary(
        pool: &PgPool,
        user_id: i32,
        foreign: &str,
    ) -> Result<Vec<NbackSummary>, WisecrowError> {
        let rows: Vec<(String, i16, i64)> = sqlx::query_as(
            "SELECT native_lang, MAX(n_level_peak), COUNT(*)
             FROM dnb_sessions
             WHERE user_id = $1 AND foreign_lang = $2
               AND completed_at IS NOT NULL AND trials_completed > 0
             GROUP BY native_lang
             ORDER BY native_lang",
        )
        .bind(user_id)
        .bind(foreign)
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(native, highest_n, sessions)| NbackSummary {
                native,
                highest_n,
                sessions,
            })
            .collect())
    }

    /// Rolls up everything the dashboard shows for one learner, one entry per
    /// foreign language in name order.
    ///
    /// # Errors
    ///
    /// Returns an error when any query fails.
    pub async fn dashboard(
        pool: &PgPool,
        user_id: i32,
    ) -> Result<Vec<LanguageProgress>, WisecrowError> {
        let scopes = Self::languages_for_user(pool, user_id).await?;
        let mut languages = Vec::with_capacity(scopes.len());
        for scope in scopes {
            let mut vocabulary = Vec::with_capacity(scope.natives.len());
            for native in scope.natives {
                let counts =
                    Self::vocabulary_counts(pool, user_id, &native, &scope.foreign).await?;
                let reviews =
                    Self::daily_reviews(pool, user_id, &native, &scope.foreign, REVIEW_DAYS)
                        .await?;
                let verb_forms = verb_form_progress(pool, &native, &scope.foreign).await?;
                vocabulary.push(PairVocabulary {
                    native,
                    counts,
                    reviews,
                    verb_forms,
                });
            }
            let grammar = Self::grammar_bands(pool, user_id, &scope.foreign).await?;
            let placement = Self::placement_summary(pool, user_id, &scope.foreign).await?;
            let nback = Self::nback_summary(pool, user_id, &scope.foreign).await?;
            let books = book_progress(pool, user_id, &scope.foreign).await?;
            languages.push(LanguageProgress {
                foreign: scope.foreign,
                foreign_name: scope.foreign_name,
                vocabulary,
                grammar,
                placement,
                nback,
                books,
            });
        }
        Ok(languages)
    }
}

/// Counts books this learner has started in one foreign language, and how many
/// of those files are finished.
///
/// A place is stored against the book's source language, so the count joins
/// that language rather than treating the position as the language being
/// learned. A file is finished when a ready book for it has no failed run and
/// its last sentence is the saved place.
///
/// # Errors
///
/// Returns an error when the query fails or a count does not fit the tile.
async fn book_progress(
    pool: &PgPool,
    user_id: i32,
    foreign: &str,
) -> Result<wisecrow_dto::BookProgressDto, WisecrowError> {
    let (started, finished): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*)::bigint,
                COUNT(*) FILTER (WHERE EXISTS (
                    SELECT 1
                    FROM books b
                    JOIN book_sentences last_sentence ON last_sentence.book_id = b.id
                    JOIN languages fl ON fl.code = $2
                    WHERE b.file = rp.file
                      AND b.source_language_id = rp.language_id
                      AND b.status = 'ready'
                      AND last_sentence.position = (
                          SELECT MAX(s.position) FROM book_sentences s WHERE s.book_id = b.id
                      )
                      AND last_sentence.chapter = rp.chapter
                      AND last_sentence.paragraph = rp.paragraph
                      AND last_sentence.sentence = rp.sentence
                      AND COALESCE((
                          SELECT cardinality(p.failed_runs)
                          FROM book_import_progress p
                          WHERE p.book_id = b.id AND p.language_id = fl.id
                      ), 0) = 0
                ))::bigint
         FROM reading_positions rp
         WHERE rp.user_id = $1
           AND EXISTS (
               SELECT 1
               FROM books b
               JOIN book_sentences s ON s.book_id = b.id
               JOIN book_translations t ON t.sentence_id = s.id
               JOIN languages fl ON fl.id = t.language_id AND fl.code = $2
               WHERE b.file = rp.file
                 AND b.source_language_id = rp.language_id
           )",
    )
    .bind(user_id)
    .bind(foreign)
    .fetch_one(pool)
    .await?;
    let started = u32::try_from(started)
        .map_err(|_| WisecrowError::InvalidInput("a reader has too many books".into()))?;
    let finished = u32::try_from(finished)
        .map_err(|_| WisecrowError::InvalidInput("a reader has too many books".into()))?;
    Ok(wisecrow_dto::BookProgressDto { started, finished })
}

/// Counts stored verb forms for one pair.
///
/// `voiced` is the number of forms whose spelling has an audio row, not the
/// number of distinct clips. Two slots that spell the same form and share one
/// clip each count.
///
/// # Errors
///
/// Returns an error when the query fails.
pub async fn verb_form_progress(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
) -> Result<wisecrow_dto::VerbFormProgressDto, WisecrowError> {
    let row: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
             (SELECT count(DISTINCT vf.lemma)
              FROM verb_forms vf
              JOIN languages l ON l.id = vf.language_id
              JOIN verb_paradigm_runs r
                ON r.language_id = vf.language_id AND r.lemma = vf.lemma
               AND r.paradigm_version = vf.paradigm_version AND r.status = 'ok'
              WHERE l.code = $2),
             (SELECT count(*)
              FROM verb_form_translations vft
              JOIN verb_forms vf ON vf.id = vft.verb_form_id
              JOIN languages nl ON nl.id = vft.native_language_id
              JOIN languages fl ON fl.id = vf.language_id
              WHERE nl.code = $1 AND fl.code = $2),
             (SELECT count(*)
              FROM verb_form_translations vft
              JOIN verb_forms vf ON vf.id = vft.verb_form_id
              JOIN languages nl ON nl.id = vft.native_language_id
              JOIN languages fl ON fl.id = vf.language_id
              WHERE nl.code = $1 AND fl.code = $2 AND vf.attested),
             (SELECT count(*)
              FROM verb_form_translations vft
              JOIN verb_forms vf ON vf.id = vft.verb_form_id
              JOIN languages nl ON nl.id = vft.native_language_id
              JOIN languages fl ON fl.id = vf.language_id
              WHERE nl.code = $1 AND fl.code = $2
                AND EXISTS (
                    SELECT 1 FROM sentence_media sm
                    WHERE sm.language_code = fl.code
                      AND sm.media_type = 'audio'
                      AND sm.sentence = vf.form
                ))",
    )
    .bind(native_lang)
    .bind(foreign_lang)
    .fetch_one(pool)
    .await?;
    Ok(wisecrow_dto::VerbFormProgressDto {
        lemmas: row.0,
        forms: row.1,
        attested: row.2,
        voiced: row.3,
    })
}

/// Folds `(foreign, native)` rows, sorted by foreign then native, into one
/// scope per foreign language, ordered by the name a learner reads.
fn group_languages(rows: Vec<(String, Option<String>)>) -> Vec<LanguageScope> {
    let mut scopes: Vec<LanguageScope> = Vec::new();
    for (foreign, native) in rows {
        match scopes.last_mut() {
            Some(scope) if scope.foreign == foreign => scope.natives.extend(native),
            _ => scopes.push(LanguageScope {
                foreign_name: language_name(&foreign),
                foreign,
                natives: native.into_iter().collect(),
            }),
        }
    }
    scopes.sort_by(|left, right| {
        left.foreign_name
            .cmp(&right.foreign_name)
            .then_with(|| left.foreign.cmp(&right.foreign))
    });
    scopes
}

/// The name a learner recognises, falling back to the code for a language the
/// supported table does not list.
fn language_name(code: &str) -> String {
    crate::cli::SUPPORTED_LANGUAGE_INFO
        .iter()
        .find(|(candidate, _)| *candidate == code)
        .map_or_else(|| code.to_owned(), |(_, name)| (*name).to_owned())
}

/// Lays counted days onto a run of `days` days ending today, oldest first.
fn zero_fill(today: NaiveDate, days: u32, counted: &[(NaiveDate, i64)]) -> Vec<DailyCount> {
    (0..days)
        .rev()
        .filter_map(|back| today.checked_sub_days(Days::new(u64::from(back))))
        .map(|day| DailyCount {
            day,
            count: counted
                .iter()
                .find(|(counted_day, _)| *counted_day == day)
                .map_or(0, |(_, count)| *count),
        })
        .collect()
}

/// Counts mastery rows per level, in the syllabus order the rows arrive in.
fn count_bands(rows: &[MasteryRow]) -> Vec<LevelBands> {
    let mut levels: Vec<LevelBands> = Vec::new();
    for row in rows {
        let attempts = usize::try_from(row.attempts).unwrap_or(0);
        let band = band_from(row.accuracy.unwrap_or(0.0), attempts);
        match levels.last_mut() {
            Some(current) if current.level == row.level => current.count(band),
            _ => {
                let mut fresh = LevelBands {
                    level: row.level.clone(), // clone: the tally owns its level
                    ..LevelBands::default()
                };
                fresh.count(band);
                levels.push(fresh);
            }
        }
    }
    levels
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use super::*;

    fn row(foreign: &str, native: Option<&str>) -> (String, Option<String>) {
        (foreign.to_owned(), native.map(str::to_owned))
    }

    #[test]
    fn rows_fold_into_one_scope_per_foreign_language_ordered_by_name() {
        let scopes = group_languages(vec![
            row("es", None),
            row("es", Some("cy")),
            row("es", Some("en")),
            row("gd", None),
            row("zqxx", Some("en")),
        ]);

        assert_eq!(
            scopes,
            vec![
                LanguageScope {
                    foreign: "gd".into(),
                    foreign_name: "Scottish Gaelic".into(),
                    natives: Vec::new(),
                },
                LanguageScope {
                    foreign: "es".into(),
                    foreign_name: "Spanish".into(),
                    natives: vec!["cy".into(), "en".into()],
                },
                LanguageScope {
                    foreign: "zqxx".into(),
                    foreign_name: "zqxx".into(),
                    natives: vec!["en".into()],
                },
            ]
        );
    }

    #[test]
    fn an_unlisted_code_is_named_by_its_code() {
        assert_eq!(language_name("es"), "Spanish");
        assert_eq!(language_name("zqxx"), "zqxx");
    }

    #[test]
    fn the_strip_has_one_entry_per_day_oldest_first_with_gaps_as_zero() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 20).expect("date");
        let day = |d| NaiveDate::from_ymd_opt(2026, 10, d).expect("date");
        let counted = [(day(10), 9), (day(18), 3), (today, 2)];

        assert_eq!(
            zero_fill(today, 3, &counted),
            vec![
                DailyCount {
                    day: day(18),
                    count: 3
                },
                DailyCount {
                    day: day(19),
                    count: 0
                },
                DailyCount {
                    day: today,
                    count: 2
                },
            ],
            "a day before the window is not shown"
        );
        assert_eq!(zero_fill(today, REVIEW_DAYS, &counted).len(), 30);
        assert!(zero_fill(today, 0, &counted).is_empty());
    }

    fn mastery(level: &str, accuracy: Option<f64>, attempts: i32) -> MasteryRow {
        MasteryRow {
            rule_id: 0,
            slug: String::new(),
            title: String::new(),
            level: level.to_owned(),
            source: String::from("llm"),
            accuracy,
            attempts,
            reps: 0,
            state: 0,
            due: None,
        }
    }

    #[test]
    fn bands_are_counted_per_level_in_syllabus_order() {
        let rows = [
            mastery("A1", Some(1.0), 3),
            mastery("A1", Some(0.0), 2),
            mastery("A1", None, 0),
            mastery("A2", Some(0.7), 5),
            mastery("B1", Some(0.5), 3),
            mastery("B1", Some(0.9), 1),
            mastery("B1", Some(0.65), 2),
        ];

        assert_eq!(
            count_bands(&rows),
            vec![
                LevelBands {
                    level: "A1".into(),
                    green: 1,
                    provisional_red: 1,
                    unseen: 1,
                    ..LevelBands::default()
                },
                LevelBands {
                    level: "A2".into(),
                    amber: 1,
                    ..LevelBands::default()
                },
                LevelBands {
                    level: "B1".into(),
                    red: 1,
                    provisional_green: 1,
                    provisional_amber: 1,
                    ..LevelBands::default()
                },
            ]
        );
    }

    #[test]
    fn an_unattempted_point_is_unseen_whatever_accuracy_it_carries() {
        assert_eq!(
            count_bands(&[mastery("C2", Some(1.0), 0)]),
            vec![LevelBands {
                level: "C2".into(),
                unseen: 1,
                ..LevelBands::default()
            }]
        );
    }
}
