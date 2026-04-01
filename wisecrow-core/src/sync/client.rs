use reqwest::Client;
use sqlx::PgPool;
use url::Url;

use crate::errors::WisecrowError;
use crate::grammar::rules::{NewGrammarRule, NewRuleExample, RuleRepository, RuleSource};
use crate::ingesting::persisting::DatabasePersister;
use wisecrow_dto::{SyncGrammarRuleDto, SyncLanguageDto, SyncTranslationDto};

pub struct SyncClient {
    client: Client,
    base_url: Url,
}

impl SyncClient {
    /// Creates a new sync client for the given remote URL.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL is invalid or the HTTP client cannot be built.
    pub fn new(remote_url: &str, api_key: Option<&str>) -> Result<Self, WisecrowError> {
        let base_url = Url::parse(remote_url)?;
        let mut builder = Client::builder();
        if let Some(key) = api_key {
            let mut headers = reqwest::header::HeaderMap::new();
            let val = reqwest::header::HeaderValue::from_str(key)
                .map_err(|e| WisecrowError::InvalidInput(format!("Invalid API key header: {e}")))?;
            headers.insert("x-api-key", val);
            builder = builder.default_headers(headers);
        }
        let client = builder.build()?;
        Ok(Self { client, base_url })
    }

    fn endpoint_url(&self, path: &str) -> Result<Url, WisecrowError> {
        self.base_url
            .join(path)
            .map_err(|e| WisecrowError::SyncError(format!("URL join failed: {e}")))
    }

    /// Syncs all languages from the remote, returns count of synced items.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request or database upsert fails.
    pub async fn sync_languages(&self, pool: &PgPool) -> Result<usize, WisecrowError> {
        let mut after_id = 0i32;
        let mut total = 0usize;
        let persister = DatabasePersister::new(pool.clone()); // clone: PgPool is Arc-based

        loop {
            let url = self.endpoint_url("/api/sync_languages")?;
            let langs: Vec<SyncLanguageDto> = self
                .client
                .get(url)
                .query(&[("after_id", after_id)])
                .send()
                .await
                .map_err(|e| WisecrowError::SyncError(format!("Sync request failed: {e}")))?
                .json()
                .await
                .map_err(|e| WisecrowError::SyncError(format!("Sync parse failed: {e}")))?;

            if langs.is_empty() {
                break;
            }

            for lang in &langs {
                persister.ensure_language(&lang.code, &lang.name).await?;
                after_id = lang.id;
            }

            total = total.saturating_add(langs.len());
        }

        Ok(total)
    }

    /// Syncs all translations from the remote, returns count of synced items.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request or database upsert fails.
    pub async fn sync_translations(&self, pool: &PgPool) -> Result<usize, WisecrowError> {
        let mut after_id = 0i32;
        let mut total = 0usize;
        let persister = DatabasePersister::new(pool.clone()); // clone: PgPool is Arc-based

        loop {
            let url = self.endpoint_url("/api/sync_translations")?;
            let translations: Vec<SyncTranslationDto> = self
                .client
                .get(url)
                .query(&[("after_id", after_id)])
                .send()
                .await
                .map_err(|e| WisecrowError::SyncError(format!("Sync request failed: {e}")))?
                .json()
                .await
                .map_err(|e| WisecrowError::SyncError(format!("Sync parse failed: {e}")))?;

            if translations.is_empty() {
                break;
            }

            for t in &translations {
                let from_id = persister
                    .ensure_language(&t.from_language_code, &t.from_language_code)
                    .await?;
                let to_id = persister
                    .ensure_language(&t.to_language_code, &t.to_language_code)
                    .await?;

                sqlx::query(
                    "INSERT INTO translations (from_language_id, from_phrase, to_language_id, to_phrase, frequency)
                     VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
                     DO UPDATE SET frequency = GREATEST(translations.frequency, EXCLUDED.frequency)",
                )
                .bind(from_id)
                .bind(&t.from_phrase)
                .bind(to_id)
                .bind(&t.to_phrase)
                .bind(t.frequency)
                .execute(pool)
                .await?;

                after_id = t.id;
            }

            total = total.saturating_add(translations.len());
        }

        Ok(total)
    }

    /// Syncs all grammar rules from the remote, returns count of synced items.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request or database upsert fails.
    pub async fn sync_grammar_rules(&self, pool: &PgPool) -> Result<usize, WisecrowError> {
        let mut after_id = 0i32;
        let mut total = 0usize;
        let persister = DatabasePersister::new(pool.clone()); // clone: PgPool is Arc-based

        loop {
            let url = self.endpoint_url("/api/sync_grammar_rules")?;
            let rules: Vec<SyncGrammarRuleDto> = self
                .client
                .get(url)
                .query(&[("after_id", after_id)])
                .send()
                .await
                .map_err(|e| WisecrowError::SyncError(format!("Sync request failed: {e}")))?
                .json()
                .await
                .map_err(|e| WisecrowError::SyncError(format!("Sync parse failed: {e}")))?;

            if rules.is_empty() {
                break;
            }

            for rule in &rules {
                let lang_id = persister
                    .ensure_language(&rule.language_code, &rule.language_code)
                    .await?;
                let cefr_level_id =
                    RuleRepository::ensure_cefr_level(pool, &rule.cefr_level_code).await?;

                let source = rule
                    .source
                    .parse::<RuleSource>()
                    .unwrap_or(RuleSource::Manual);
                let new_rule = NewGrammarRule {
                    title: rule.title.clone(), // clone: building owned from borrowed sync data
                    explanation: rule.explanation.clone(), // clone: building owned from borrowed sync data
                    source,
                    examples: rule
                        .examples
                        .iter()
                        .map(|ex| NewRuleExample {
                            sentence: ex.sentence.clone(), // clone: building owned from borrowed sync data
                            translation: ex.translation.clone(), // clone: building owned from borrowed sync data
                            is_correct: ex.is_correct,
                        })
                        .collect(),
                };

                RuleRepository::upsert_rule(pool, lang_id, cefr_level_id, &new_rule).await?;
                after_id = rule.id;
            }

            total = total.saturating_add(rules.len());
        }

        Ok(total)
    }
}
