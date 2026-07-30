//! Removing corpus junk from pairs that are already stored.
//!
//! [`crate::ingesting::parsing::CorpusParser::send_pair`] keeps this junk out
//! at ingest time, but a corpus ingested before that guard existed still holds
//! it. Rather than express the rules a second time in SQL — where the two
//! copies drift silently — this reads the phrases back and applies the very
//! functions the parser and the frequency ranking use.

use crate::errors::WisecrowError;
use sqlx::PgPool;

/// Rows read per page. Matches the frequency ranking's paging, so a corpus of
/// any size is walked without being held in memory.
const PAGE_SIZE: i64 = 5000;

/// Rows updated or removed per statement.
const BATCH_SIZE: usize = 1000;

/// What a prune did, or would do.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PruneReport {
    pub scanned: usize,
    /// Pairs whose phrase is not in the language's script at all.
    pub deleted: usize,
    /// Pairs demoted to frequency 1 because the phrase is one unsegmented run.
    pub demoted: usize,
}

pub struct Pruner;

impl Pruner {
    /// Removes stored pairs that the ingest guards would now reject.
    ///
    /// A phrase in the wrong script is deleted: it is not the language it
    /// claims to be, and nothing downstream can use it. A phrase that is one
    /// over-long unsegmented run is only demoted to frequency 1, which is what
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
        dry_run: bool,
    ) -> Result<PruneReport, WisecrowError> {
        if !crate::lang::is_valid_code(lang_code) {
            return Err(WisecrowError::InvalidInput(format!(
                "Invalid language code: {lang_code}"
            )));
        }
        let mut report = PruneReport::default();
        for column in ["from", "to"] {
            Self::prune_side(pool, lang_code, column, dry_run, &mut report).await?;
        }
        Ok(report)
    }

    async fn prune_side(
        pool: &PgPool,
        lang_code: &str,
        column: &str,
        dry_run: bool,
        report: &mut PruneReport,
    ) -> Result<(), WisecrowError> {
        // `column` is chosen from a literal array above, never from user input.
        let statement = format!(
            "SELECT t.id, t.{column}_phrase, t.frequency
               FROM translations t
               JOIN languages l ON l.id = t.{column}_language_id
              WHERE l.code = $1 AND t.id > $2
              ORDER BY t.id
              LIMIT $3"
        );

        let mut after_id = 0i32;
        loop {
            let rows: Vec<(i32, String, i32)> = sqlx::query_as(&statement)
                .bind(lang_code)
                .bind(after_id)
                .bind(PAGE_SIZE)
                .fetch_all(pool)
                .await?;

            let Some((last_id, _, _)) = rows.last() else {
                return Ok(());
            };
            after_id = *last_id;
            report.scanned += rows.len();

            let (doomed, demoted) = Self::classify(&rows, lang_code);
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
    /// A wrong-script phrase is never also considered for demotion: it is going
    /// regardless, and counting it twice would overstate the report.
    fn classify(rows: &[(i32, String, i32)], lang_code: &str) -> (Vec<i32>, Vec<i32>) {
        let mut doomed = Vec::new();
        let mut demoted = Vec::new();
        for (id, phrase, frequency) in rows {
            if !crate::lang::is_plausible_script(phrase, lang_code) {
                doomed.push(*id);
            } else if *frequency > 1 && crate::lang::is_unsegmented_run(phrase) {
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
            sqlx::query("UPDATE translations SET frequency = 1 WHERE id = ANY($1)")
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

    fn row(id: i32, phrase: &str, frequency: i32) -> (i32, String, i32) {
        (id, phrase.to_owned(), frequency)
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

        let (doomed, demoted) = Pruner::classify(&rows, "gd");

        assert_eq!(doomed, vec![2], "only the kana blob is not Gaelic");
        assert_eq!(
            demoted,
            vec![3],
            "the Latin run is Gaelic-shaped but is one 122-character token"
        );
    }

    #[test]
    fn leaves_an_already_demoted_run_alone() {
        // Nothing to do for a row that cannot reach a deck: `unlearned` filters
        // on frequency > 1, so re-demoting it would be a write for no change.
        let rows = vec![row(1, &"i".repeat(122), 1)];

        let (doomed, demoted) = Pruner::classify(&rows, "gd");

        assert!(doomed.is_empty());
        assert!(demoted.is_empty());
    }

    #[test]
    fn a_language_it_knows_nothing_about_loses_nothing_to_the_script_rule() {
        let rows = vec![row(1, "うぐぅ", 5)];

        let (doomed, _) = Pruner::classify(&rows, "xx");

        assert!(doomed.is_empty());
    }
}
