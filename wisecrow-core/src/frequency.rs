use crate::errors::WisecrowError;
use reqwest::Client;
use sqlx::PgPool;
use std::collections::HashMap;
use std::time::Duration;
use url::Url;

const HERMIT_DAVE_BASE: &str =
    "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/";
const BATCH_SIZE: usize = 1000;
/// Rows read per page when deriving counts from the stored corpus.
const PHRASE_PAGE_SIZE: usize = 5000;

/// Characters stripped from the edges of a phrase before it is compared with a
/// frequency list. Corpus phrases carry sentence punctuation that the lists do
/// not, so "O." would otherwise never match "o".
///
/// This set must stay in step with the `btrim` argument in migration
/// `016_normalised_phrase_indexes.sql` and in [`FrequencyUpdater::bulk_update`];
/// a mismatch silently costs the index rather than failing loudly.
const MATCH_TRIM_CHARS: &[char] = &['.', ',', '!', '?', ';', ':', '"', '\'', '¡', '¿'];

/// [`MATCH_TRIM_CHARS`] as the body of a SQL string literal, with the single
/// quote doubled. `trim_set_is_consistent_across_rust_sql_and_migration` holds
/// the three copies together.
const MATCH_TRIM_SQL: &str = ".,!?;:\"''¡¿";

pub struct FrequencyUpdater;

impl FrequencyUpdater {
    /// Downloads the Hermit Dave frequency list for `lang_code` and updates
    /// `translations.frequency` for matching `from_phrase` entries.
    ///
    /// # Errors
    ///
    /// Returns an error if the download or database update fails.
    pub async fn update_from_hermit_dave(
        pool: &PgPool,
        lang_code: &str,
    ) -> Result<usize, WisecrowError> {
        if !crate::lang::is_valid_code(lang_code) {
            return Err(WisecrowError::InvalidInput(format!(
                "Invalid language code: {lang_code}"
            )));
        }
        let base = Url::parse(HERMIT_DAVE_BASE)?;
        let url = base.join(&format!("{lang_code}/{lang_code}_50k.txt"))?;
        let client = Client::builder().timeout(Duration::from_secs(60)).build()?;
        let response = client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(WisecrowError::InvalidInput(format!(
                "Failed to fetch frequency list: HTTP {}",
                response.status()
            )));
        }

        let body = response.text().await?;
        let frequencies = Self::parse_frequency_text(&body);
        Self::bulk_update(pool, lang_code, &frequencies).await
    }

    /// Derives a word-frequency list from the phrases already stored for
    /// `lang_code` and applies it, which is the route for languages nobody has
    /// published a list for. Counts come from the material actually being
    /// studied rather than from a corpus of another genre.
    ///
    /// Note that this changes where the counts come from, not what they can
    /// rank: matching remains whole-phrase, so single-word rows are ranked and
    /// the sentences containing those words are not.
    ///
    /// # Errors
    ///
    /// Returns an error if the language is unknown, has no tokeniser, or if a
    /// database query fails.
    pub async fn update_from_corpus(
        pool: &PgPool,
        lang_code: &str,
    ) -> Result<usize, WisecrowError> {
        if !crate::lang::is_valid_code(lang_code) {
            return Err(WisecrowError::InvalidInput(format!(
                "Invalid language code: {lang_code}"
            )));
        }
        let tokenizer = crate::preview::tokenize::for_language(lang_code)?;

        let mut counts: HashMap<String, i32> = HashMap::new();
        for column in ["from", "to"] {
            Self::count_side(pool, lang_code, column, tokenizer.as_ref(), &mut counts).await?;
        }

        if counts.is_empty() {
            return Err(WisecrowError::InvalidInput(format!(
                "No phrases stored for {lang_code}, so no frequencies could be derived. \
                 Ingest a corpus for it first."
            )));
        }
        tracing::info!(
            "Derived {} distinct {lang_code} word forms from the stored corpus",
            counts.len()
        );
        Self::bulk_update(pool, lang_code, &counts).await
    }

    /// Tokenises one side of the stored pairs, paging on the primary key so
    /// that a multi-million-row corpus never has to be held in memory at once.
    async fn count_side(
        pool: &PgPool,
        lang_code: &str,
        column: &str,
        tokenizer: &dyn crate::preview::tokenize::Tokenizer,
        counts: &mut HashMap<String, i32>,
    ) -> Result<(), WisecrowError> {
        // `column` is chosen from a literal array above, never from user input.
        let statement = format!(
            "SELECT t.id, t.{column}_phrase
               FROM translations t
               JOIN languages l ON l.id = t.{column}_language_id
              WHERE l.code = $1 AND t.id > $2
              ORDER BY t.id
              LIMIT $3"
        );

        let mut after_id = 0i32;
        loop {
            let rows: Vec<(i32, String)> = sqlx::query_as(&statement)
                .bind(lang_code)
                .bind(after_id)
                .bind(i64::try_from(PHRASE_PAGE_SIZE).unwrap_or(i64::MAX))
                .fetch_all(pool)
                .await?;

            let Some((last_id, _)) = rows.last() else {
                return Ok(());
            };
            after_id = *last_id;

            for (_, phrase) in &rows {
                for token in tokenizer.tokenize(phrase) {
                    Self::accumulate(counts, token, 1);
                }
            }
        }
    }

    /// Updates `translations.frequency` from a local file in `word count`
    /// format (one entry per line, space-separated).
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or the database update
    /// fails.
    pub async fn update_from_file(
        pool: &PgPool,
        lang_code: &str,
        path: &str,
    ) -> Result<usize, WisecrowError> {
        let content = std::fs::read_to_string(path)?;
        let frequencies = Self::parse_frequency_text(&content);
        if frequencies.is_empty() {
            return Err(WisecrowError::InvalidInput(format!(
                "No frequency entries parsed from {path}. Expected one `word count` \
                 pair per line, or the Leipzig `rank<TAB>word<TAB>count` layout."
            )));
        }
        Self::bulk_update(pool, lang_code, &frequencies).await
    }

    /// Splits a Leipzig Corpora Collection word line, which is
    /// `rank<TAB>word<TAB>count`. Returning `None` for anything else keeps a
    /// stray tab in a `word count` list from being read as a Leipzig row.
    fn split_leipzig_line(line: &str) -> Option<(&str, &str)> {
        let mut fields = line.split('\t');
        let rank = fields.next()?;
        let word = fields.next()?;
        let count = fields.next()?;
        if fields.next().is_some() || rank.parse::<u64>().is_err() {
            return None;
        }
        Some((word, count))
    }

    /// Splits a two-column CSV line, `word,count`. A third empty field is
    /// tolerated because published lists often carry a trailing comma.
    fn split_csv_line(line: &str) -> Option<(&str, &str)> {
        let mut fields = line.split(',');
        let word = fields.next()?;
        let count = fields.next()?;
        Some((word, count))
    }

    /// Reads one line under whichever layout fits, in order of specificity.
    /// A candidate is accepted only once its count parses, so a `word count`
    /// line containing a comma is still read as space-separated rather than
    /// being mangled by the CSV rule.
    fn extract_entry(line: &str) -> Option<(&str, i32)> {
        let candidates = [
            Self::split_leipzig_line(line),
            line.rsplit_once(' '),
            Self::split_csv_line(line),
        ];
        for (word, count) in candidates.into_iter().flatten() {
            if let Ok(count) = count.trim().parse::<i64>() {
                let clamped = i32::try_from(count.min(i64::from(i32::MAX))).unwrap_or(i32::MAX);
                return Some((word.trim(), clamped));
            }
        }
        None
    }

    fn parse_frequency_text(text: &str) -> HashMap<String, i32> {
        let mut map = HashMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((word, count)) = Self::extract_entry(line) {
                Self::accumulate(&mut map, word.to_owned(), count);
            }
        }
        map
    }

    fn accumulate(map: &mut HashMap<String, i32>, word: String, count: i32) {
        map.entry(word)
            .and_modify(|existing: &mut i32| *existing = existing.saturating_add(count))
            .or_insert(count);
    }

    /// Lower-cases each listed word and strips edge punctuation so it can be
    /// compared with the normalised phrase columns. Words that collide once
    /// normalised have their counts summed.
    fn normalise_keys(frequencies: &HashMap<String, i32>) -> HashMap<String, i32> {
        let mut normalised = HashMap::with_capacity(frequencies.len());
        for (word, count) in frequencies {
            let key = word.trim_matches(MATCH_TRIM_CHARS).to_lowercase();
            if key.is_empty() {
                continue;
            }
            normalised
                .entry(key)
                .and_modify(|existing: &mut i32| *existing = existing.saturating_add(*count))
                .or_insert(*count);
        }
        normalised
    }

    async fn bulk_update(
        pool: &PgPool,
        lang_code: &str,
        frequencies: &HashMap<String, i32>,
    ) -> Result<usize, WisecrowError> {
        let lang_id: Option<i32> = sqlx::query_scalar("SELECT id FROM languages WHERE code = $1")
            .bind(lang_code)
            .fetch_optional(pool)
            .await?;

        let Some(lang_id) = lang_id else {
            tracing::warn!(
                "Language {lang_code} is not in the database, so no frequencies were applied; \
                 ingest a corpus for it first"
            );
            return Ok(0);
        };

        let normalised = Self::normalise_keys(frequencies);
        let entries: Vec<(&String, &i32)> = normalised.iter().collect();
        let mut total_updated = 0usize;

        // Either side may be the listed language: a pair ingested as
        // `-n en -f cy` holds Welsh in to_phrase, and a Welsh list should rank
        // it. The btrim sets match migration 016, so both expression indexes
        // are usable.
        let statement = format!(
            "UPDATE translations SET frequency = t.freq
             FROM unnest($1::text[], $2::int[]) AS t(phrase, freq)
             WHERE (translations.from_language_id = $3
                    AND lower(btrim(translations.from_phrase, '{MATCH_TRIM_SQL}')) = t.phrase)
                OR (translations.to_language_id = $3
                    AND lower(btrim(translations.to_phrase, '{MATCH_TRIM_SQL}')) = t.phrase)"
        );

        for chunk in entries.chunks(BATCH_SIZE) {
            let words: Vec<&str> = chunk.iter().map(|(w, _)| w.as_str()).collect();
            let counts: Vec<i32> = chunk.iter().map(|(_, c)| **c).collect();

            let result = sqlx::query(&statement)
                .bind(&words)
                .bind(&counts)
                .bind(lang_id)
                .execute(pool)
                .await?;

            total_updated = total_updated
                .saturating_add(usize::try_from(result.rows_affected()).unwrap_or(usize::MAX));
        }

        tracing::info!("Updated {total_updated} frequency entries for {lang_code}");
        Ok(total_updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use rstest::rstest;

    #[rstest]
    #[case("hello 12345\nworld 6789\n", &[("hello", 12345), ("world", 6789)])]
    #[case("\nhello 100\n\n\nworld 200\n", &[("hello", 100), ("world", 200)])]
    #[case("hello 100\nbadline\nworld 200\nnumber abc\n", &[("hello", 100), ("world", 200)])]
    #[case("ice cream 500\nhello world 100\n", &[("ice cream", 500), ("hello world", 100)])]
    #[case("1\tyn\t102513\n2\ty\t95221\n", &[("yn", 102513), ("y", 95221)])]
    #[case("1\tyn\t102513\nhello 100\n", &[("yn", 102513), ("hello", 100)])]
    #[case("yn\t102513\n1\tyn\tabc\n1\tyn\t5\textra\n", &[])]
    #[case("a,75006,\nan,67766,\n", &[("a", 75006), ("an", 67766)])]
    #[case("a,75006\nice cream,500\n", &[("a", 75006), ("ice cream", 500)])]
    #[case("hello, world 100\n", &[("hello, world", 100)])]
    #[case("word,notanumber\nhello,world\n", &[])]
    fn parse_frequency_text_cases(#[case] input: &str, #[case] expected: &[(&str, i32)]) {
        let map = FrequencyUpdater::parse_frequency_text(input);
        assert_eq!(map.len(), expected.len());
        for (word, count) in expected {
            assert_eq!(map.get(*word), Some(count));
        }
    }

    #[test]
    fn trim_set_is_consistent_across_rust_sql_and_migration() {
        let from_chars: String = MATCH_TRIM_CHARS
            .iter()
            .collect::<String>()
            .replace('\'', "''");
        assert_eq!(
            from_chars, MATCH_TRIM_SQL,
            "the SQL literal must spell out MATCH_TRIM_CHARS with the quote doubled"
        );

        let migration = include_str!("../migrations/016_normalised_phrase_indexes.sql");
        for column in ["from_phrase", "to_phrase"] {
            assert!(
                migration.contains(&format!("btrim({column}, '{MATCH_TRIM_SQL}')")),
                "migration 016 must index the same normalisation used to match {column}"
            );
        }
    }

    #[rstest]
    #[case("Beth", "beth")]
    #[case("O.", "o")]
    #[case("\"Iawn!\"", "iawn")]
    #[case("¿Qué?", "qué")]
    #[case("ice cream", "ice cream")]
    fn normalisation_folds_case_and_edge_punctuation(#[case] word: &str, #[case] expected: &str) {
        let input = HashMap::from([(word.to_owned(), 7)]);
        let normalised = FrequencyUpdater::normalise_keys(&input);
        assert_eq!(normalised.get(expected), Some(&7));
    }

    #[test]
    fn normalisation_sums_colliding_words_and_drops_empty_keys() {
        let input = HashMap::from([
            ("Beth".to_owned(), 10),
            ("beth".to_owned(), 5),
            ("...".to_owned(), 99),
        ]);
        let normalised = FrequencyUpdater::normalise_keys(&input);
        assert_eq!(normalised.get("beth"), Some(&15));
        assert_eq!(normalised.len(), 1, "punctuation-only entries are dropped");
    }

    #[test]
    fn parse_duplicate_takes_sum() {
        let text = "hello 100\nhello 200\n";
        let map = FrequencyUpdater::parse_frequency_text(text);
        assert_eq!(map.get("hello"), Some(&300));
    }

    #[test]
    fn parse_large_count_clamps() {
        let text = format!("hello {}\n", i64::from(i32::MAX) + 1);
        let map = FrequencyUpdater::parse_frequency_text(&text);
        assert_eq!(map.get("hello"), Some(&i32::MAX));
    }

    proptest! {
        #[test]
        fn never_panics_on_arbitrary_input(text in ".*") {
            let _ = FrequencyUpdater::parse_frequency_text(&text);
        }

        #[test]
        fn all_values_positive_and_within_i32(
            words in prop::collection::vec("[a-z]{1,10}", 1..20),
            counts in prop::collection::vec(1i32..100_000, 1..20),
        ) {
            let len = words.len().min(counts.len());
            let text: String = words[..len]
                .iter()
                .zip(&counts[..len])
                .map(|(w, c)| format!("{w} {c}\n"))
                .collect();

            let map = FrequencyUpdater::parse_frequency_text(&text);
            for value in map.values() {
                prop_assert!(*value > 0);
            }
        }

        #[test]
        fn roundtrip_word_count_preserved(
            word in "[a-z]{1,10}",
            count in 1i32..1_000_000,
        ) {
            let text = format!("{word} {count}\n");
            let map = FrequencyUpdater::parse_frequency_text(&text);
            prop_assert_eq!(map.get(word.as_str()), Some(&count));
        }
    }
}
