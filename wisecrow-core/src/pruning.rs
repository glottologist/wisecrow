//! Removing corpus junk from pairs that are already stored.
//!
//! [`crate::ingesting::parsing::CorpusParser::send_pair`] keeps this junk out
//! at ingest time, but a corpus ingested before that guard existed still holds
//! it. Rather than express the rules a second time in SQL — where the two
//! copies drift silently — this reads the phrases back and applies the very
//! functions the parser and the frequency ranking use.

use crate::errors::WisecrowError;
use sqlx::PgPool;
use std::collections::HashSet;

/// A row as the prune reads it: its id, the phrase on the language's own side,
/// then both phrases and the corpus count.
type PrunableRow = (i32, String, String, String, Option<i32>);

/// Rows read per page. Matches the frequency ranking's paging, so a corpus of
/// any size is walked without being held in memory.
const PAGE_SIZE: i64 = 5000;

/// Rows updated or removed per statement.
const BATCH_SIZE: usize = 1000;

/// What a prune did, or would do.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PruneReport {
    pub scanned: usize,
    /// Pairs deleted: the phrase is not in the language's script, a side holds
    /// characters that render as nothing, or both sides say the same thing.
    pub deleted: usize,
    /// Pairs demoted to corpus_frequency 1: the phrase is one unsegmented run,
    /// or the native side holds no word the published list recognises.
    pub demoted: usize,
}

pub struct Pruner;

impl Pruner {
    /// Removes stored pairs that the ingest guards would now reject.
    ///
    /// A phrase in the wrong script is deleted: it is not the language it
    /// claims to be, and nothing downstream can use it. A phrase that is one
    /// over-long unsegmented run is only demoted to corpus_frequency 1, which is what
    /// [`crate::lang::MAX_WORD_CHARS`] achieves at ranking time — the row is
    /// left alone, it simply stops outranking real words.
    ///
    /// # Errors
    ///
    /// Returns an error if the language code is malformed or a database query
    /// fails.
    pub async fn run(
        pool: &PgPool,
        lang_code: &str,
        native_vocabulary: Option<&HashSet<String>>,
        dry_run: bool,
    ) -> Result<PruneReport, WisecrowError> {
        if !crate::lang::is_valid_code(lang_code) {
            return Err(WisecrowError::InvalidInput(format!(
                "Invalid language code: {lang_code}"
            )));
        }
        let mut report = PruneReport::default();
        for column in ["from", "to"] {
            Self::prune_side(
                pool,
                lang_code,
                column,
                native_vocabulary,
                dry_run,
                &mut report,
            )
            .await?;
        }
        Ok(report)
    }

    async fn prune_side(
        pool: &PgPool,
        lang_code: &str,
        column: &str,
        native_vocabulary: Option<&HashSet<String>>,
        dry_run: bool,
        report: &mut PruneReport,
    ) -> Result<(), WisecrowError> {
        // `column` is chosen from a literal array above, never from user input.
        // Both phrases are read, not just the language's own: a degenerate pair
        // is a property of the two together, and invisible characters spoil a
        // card whichever side carries them.
        let statement = format!(
            "SELECT t.id, t.{column}_phrase, t.from_phrase, t.to_phrase, t.corpus_frequency
               FROM translations t
               JOIN languages l ON l.id = t.{column}_language_id
              WHERE l.code = $1 AND t.id > $2
              ORDER BY t.id
              LIMIT $3"
        );

        let mut after_id = 0i32;
        loop {
            let rows: Vec<PrunableRow> = sqlx::query_as(&statement)
                .bind(lang_code)
                .bind(after_id)
                .bind(PAGE_SIZE)
                .fetch_all(pool)
                .await?;

            let Some((last_id, ..)) = rows.last() else {
                return Ok(());
            };
            after_id = *last_id;
            report.scanned += rows.len();

            let (doomed, demoted) = Self::classify(&rows, lang_code, column, native_vocabulary);
            report.deleted += doomed.len();
            report.demoted += demoted.len();

            if !dry_run {
                Self::delete(pool, &doomed).await?;
                Self::demote(pool, &demoted).await?;
            }
        }
    }

    /// Splits a page into the pairs to delete and the pairs to demote.
    ///
    /// A pair that fails any deletion rule is never also considered for
    /// demotion: it is going regardless, and counting it twice would overstate
    /// the report.
    fn classify(
        rows: &[PrunableRow],
        lang_code: &str,
        column: &str,
        native_vocabulary: Option<&HashSet<String>>,
    ) -> (Vec<i32>, Vec<i32>) {
        let mut doomed = Vec::new();
        let mut demoted = Vec::new();
        for (id, phrase, source, target, corpus_frequency) in rows {
            // The native phrase is whichever side is not the language being
            // pruned; that is the one a learner reads as the prompt.
            let native = if column == "to" { source } else { target };
            if !crate::lang::is_plausible_script(phrase, lang_code)
                || crate::lang::has_invisible_chars(source)
                || crate::lang::has_invisible_chars(target)
                || crate::lang::is_degenerate_pair(source, target)
            {
                doomed.push(*id);
            } else if corpus_frequency.is_some_and(|f| f > 1)
                && (crate::lang::is_unsegmented_run(phrase)
                    || native_vocabulary
                        .is_some_and(|v| !crate::lang::has_recognised_word(native, v)))
            {
                demoted.push(*id);
            }
        }
        (doomed, demoted)
    }

    async fn delete(pool: &PgPool, ids: &[i32]) -> Result<(), WisecrowError> {
        for batch in ids.chunks(BATCH_SIZE) {
            sqlx::query("DELETE FROM translations WHERE id = ANY($1)")
                .bind(batch)
                .execute(pool)
                .await?;
        }
        Ok(())
    }

    async fn demote(pool: &PgPool, ids: &[i32]) -> Result<(), WisecrowError> {
        for batch in ids.chunks(BATCH_SIZE) {
            sqlx::query("UPDATE translations SET corpus_frequency = 1 WHERE id = ANY($1)")
                .bind(batch)
                .execute(pool)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row whose English partner is unremarkable, so that only the phrase
    /// under test can decide the outcome.
    fn row(id: i32, phrase: &str, corpus_frequency: i32) -> PrunableRow {
        pair(id, "a harmless partner", phrase, Some(corpus_frequency))
    }

    /// A pair ranking has never touched, whatever its collision count.
    fn unranked_row(id: i32, phrase: &str) -> PrunableRow {
        pair(id, "a harmless partner", phrase, None)
    }

    /// The language under test is always the `to` side here, matching how the
    /// Celtic corpora were ingested (`-n en -f gd`).
    fn pair(id: i32, source: &str, target: &str, corpus_frequency: Option<i32>) -> PrunableRow {
        (
            id,
            target.to_owned(),
            source.to_owned(),
            target.to_owned(),
            corpus_frequency,
        )
    }

    #[test]
    fn deletes_wrong_script_and_demotes_unsegmented_runs() {
        let blob = "うぐぅ".repeat(32);
        let latin_run = "i".repeat(122);
        let rows = vec![
            row(1, "Halò a charaid", 400),
            row(2, &blob, 166_950),
            row(3, &latin_run, 17_596),
            row(4, "Chaidh e gu 東京 an-dè", 12),
            row(5, &latin_run, 1),
        ];

        let (doomed, demoted) = Pruner::classify(&rows, "gd", "to", None);

        assert_eq!(doomed, vec![2], "only the kana blob is not Gaelic");
        assert_eq!(
            demoted,
            vec![3],
            "the Latin run is Gaelic-shaped but is one 122-character token"
        );
    }

    #[test]
    fn deletes_pairs_that_say_the_same_thing_on_both_sides() {
        // Both reached real decks. English "An" and Irish "An" are different
        // words that happen to share a spelling, and English "Air" and Gaelic
        // "Air" likewise; a card showing one against the other teaches nothing,
        // whatever its corpus count.
        let rows = vec![
            pair(1, "An.", "An.", Some(2_899_864)),
            pair(2, "Air.", "Air", Some(5_321)),
            pair(3, "Yes.", "Tha.", Some(37_126)),
        ];

        let (doomed, demoted) = Pruner::classify(&rows, "gd", "to", None);

        assert_eq!(
            doomed,
            vec![1, 2],
            "normalisation sees through the trailing punctuation"
        );
        assert!(demoted.is_empty());
    }

    #[test]
    fn deletes_pairs_carrying_characters_that_render_as_nothing() {
        // The Irish deck served a card reading "She" whose English side ended in
        // a zero-width space, which no amount of looking at it could explain.
        let rows = vec![
            pair(1, "She\u{200B}", "Ní", Some(169_152)),
            pair(2, "Because.", "Mar.", Some(228_744)),
            pair(3, "Yes.", "T\u{FEFF}á.", Some(371_740)),
        ];

        let (doomed, _) = Pruner::classify(&rows, "ga", "to", None);

        assert_eq!(doomed, vec![1, 3], "either side spoils the pair");
    }

    #[test]
    fn demotes_pairs_whose_native_phrase_is_not_the_language_it_claims() {
        // Measured on the real Gaelic deck: "Bthey" was the *only* English
        // partner "Bha" had, and "Dthat" tied with "What?" on every statistic the
        // deck query can see, winning on the lower id. Ordering cannot separate
        // these; the corruption has to leave the data.
        let vocabulary: HashSet<String> = ["what", "yes", "the", "of", "was", "by"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let rows = vec![
            pair(1, "Bthey", "Bha", Some(6091)),
            pair(2, "andthatthe", "Dè", Some(3486)),
            pair(3, "What?", "Dè", Some(3486)),
            pair(4, "by Denk", "Rud", Some(500)),
        ];

        let (doomed, demoted) = Pruner::classify(&rows, "gd", "to", Some(&vocabulary));

        assert!(doomed.is_empty(), "a bad prompt is not bad data");
        assert_eq!(
            demoted,
            vec![1, 2],
            "one recognised word is enough to survive, so \"by Denk\" stays"
        );
    }

    #[test]
    fn without_a_vocabulary_no_pair_is_judged_on_its_native_phrase() {
        let rows = vec![pair(1, "Bthey", "Bha", Some(6091))];

        let (doomed, demoted) = Pruner::classify(&rows, "gd", "to", None);

        assert!(doomed.is_empty());
        assert!(demoted.is_empty(), "the rule is opt-in via --native-lang");
    }

    #[test]
    fn leaves_an_already_demoted_run_alone() {
        // Nothing to do for a row that cannot reach a deck: `unlearned` filters
        // on corpus_frequency > 1, so re-demoting it would be a write for no
        // change.
        let rows = vec![row(1, &"i".repeat(122), 1)];

        let (doomed, demoted) = Pruner::classify(&rows, "gd", "to", None);

        assert!(doomed.is_empty());
        assert!(demoted.is_empty());
    }

    #[test]
    fn leaves_an_unranked_run_alone() {
        // An unsegmented run ranking never scored is already outside every deck,
        // because NULL fails `corpus_frequency > 1`. Demoting it would write a
        // value where the absence of one is the more truthful state.
        let rows = vec![unranked_row(1, &"i".repeat(122))];

        let (doomed, demoted) = Pruner::classify(&rows, "gd", "to", None);

        assert!(doomed.is_empty());
        assert!(demoted.is_empty(), "NULL already fails the deck filter");
    }

    #[test]
    fn a_language_it_knows_nothing_about_loses_nothing_to_the_script_rule() {
        let rows = vec![row(1, "うぐぅ", 5)];

        let (doomed, _) = Pruner::classify(&rows, "xx", "to", None);

        assert!(doomed.is_empty());
    }
}
