use crate::errors::WisecrowError;
use reqwest::Client;
use sqlx::PgPool;
use std::collections::HashMap;
use std::time::Duration;
use url::Url;

const HERMIT_DAVE_BASE: &str =
    "https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/";
const BATCH_SIZE: usize = 1000;
const MAX_LANG_CODE_LENGTH: usize = 10;

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
        if lang_code.is_empty()
            || lang_code.len() > MAX_LANG_CODE_LENGTH
            || !lang_code.chars().all(|c| c.is_ascii_alphanumeric())
        {
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
        Self::bulk_update(pool, lang_code, &frequencies).await
    }

    fn parse_frequency_text(text: &str) -> HashMap<String, i32> {
        let mut map = HashMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some((word, count_str)) = line.rsplit_once(' ') {
                if let Ok(count) = count_str.parse::<i64>() {
                    let clamped = i32::try_from(count.min(i64::from(i32::MAX))).unwrap_or(i32::MAX);
                    map.entry(word.to_owned())
                        .and_modify(|existing: &mut i32| {
                            *existing = existing.saturating_add(clamped);
                        })
                        .or_insert(clamped);
                }
            }
        }
        map
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
            return Ok(0);
        };

        let entries: Vec<(&String, &i32)> = frequencies.iter().collect();
        let mut total_updated = 0usize;

        for chunk in entries.chunks(BATCH_SIZE) {
            let words: Vec<&str> = chunk.iter().map(|(w, _)| w.as_str()).collect();
            let counts: Vec<i32> = chunk.iter().map(|(_, c)| **c).collect();

            let result = sqlx::query(
                "UPDATE translations SET frequency = t.freq
                 FROM unnest($1::text[], $2::int[]) AS t(phrase, freq)
                 WHERE translations.from_language_id = $3
                   AND translations.from_phrase = t.phrase",
            )
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
    fn parse_frequency_text_cases(#[case] input: &str, #[case] expected: &[(&str, i32)]) {
        let map = FrequencyUpdater::parse_frequency_text(input);
        assert_eq!(map.len(), expected.len());
        for (word, count) in expected {
            assert_eq!(map.get(*word), Some(count));
        }
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
