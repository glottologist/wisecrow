//! Durable reuse of validated model responses, independent of the caller's commits.
//!
//! The table keeps the name it was created with, `grammar_llm_cache`, and its
//! key scheme, so answers cached by grammar imports before the cache served
//! other passes are still found.

use sha2::{Digest, Sha256};
use sqlx::{postgres::PgPoolOptions, PgPool};
use tracing::{info, warn};

use crate::{errors::WisecrowError, llm::LlmProvider};

// Increment when cached response validation or interpretation changes.
const CACHE_VERSION: u32 = 1;
/// How much of an unusable answer reaches the log.
const EXCERPT: usize = 240;

#[derive(Debug, thiserror::Error)]
pub enum CachedLlmError {
    #[error(transparent)]
    Response(WisecrowError),
    #[error(transparent)]
    Request(#[from] WisecrowError),
}

impl From<sqlx::Error> for CachedLlmError {
    fn from(error: sqlx::Error) -> Self {
        Self::Request(error.into())
    }
}

impl From<CachedLlmError> for WisecrowError {
    fn from(error: CachedLlmError) -> Self {
        match error {
            CachedLlmError::Request(error) | CachedLlmError::Response(error) => error,
        }
    }
}

pub struct CachedLlm<'a> {
    provider: &'a dyn LlmProvider,
    cache: Option<(PgPool, String)>,
}

impl<'a> CachedLlm<'a> {
    #[must_use]
    pub fn new(provider: &'a dyn LlmProvider, pool: Option<&PgPool>) -> Self {
        let cache = pool.zip(provider.cache_identity()).map(|(pool, identity)| {
            // Independent commits preserve paid results after a rule rollback. A
            // separate pool also works when the importer owns its only connection.
            let options = pool.connect_options().as_ref().clone(); // clone: the cache pool owns connection settings, not the importer's connections
            let pool = PgPoolOptions::new()
                .max_connections(1)
                .connect_lazy_with(options);
            (pool, identity)
        });
        Self { provider, cache }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        self.provider.name()
    }

    pub(crate) fn cache_context(&self) -> Option<(&PgPool, &str)> {
        self.cache
            .as_ref()
            .map(|(pool, identity)| (pool, identity.as_str()))
    }

    /// Returns the parsed answer to `prompt`, from the cache when a reusable
    /// answer is stored and from the provider otherwise.
    ///
    /// An answer is stored only when `reusable` accepts it, so a refused or
    /// partial answer is asked for again next time.
    ///
    /// # Errors
    ///
    /// Returns [`CachedLlmError::Response`] when `parse` rejects the answer and
    /// [`CachedLlmError::Request`] when the provider or the cache fails.
    pub async fn generate<T>(
        &self,
        prompt: &str,
        max_tokens: u32,
        parse: impl Fn(&str) -> Result<T, WisecrowError>,
        reusable: impl Fn(&T) -> bool,
    ) -> Result<T, CachedLlmError> {
        let Some((pool, identity)) = &self.cache else {
            let response = self.provider.generate(prompt, max_tokens).await?;
            return parse(&response).map_err(|error| self.unusable(error, &response));
        };
        let key = request_key(self.provider.name(), identity, prompt, max_tokens)?;
        let mut transaction = pool.begin().await?;
        let lock = i64::from_be_bytes([
            key[0], key[1], key[2], key[3], key[4], key[5], key[6], key[7],
        ]);
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock)
            .execute(&mut *transaction)
            .await?;
        let cached: Option<String> =
            sqlx::query_scalar("SELECT response FROM grammar_llm_cache WHERE request_sha256 = $1")
                .bind(key.as_slice())
                .fetch_optional(&mut *transaction)
                .await?;
        if let Some(response) = cached {
            if let Ok(parsed) = parse(&response) {
                if reusable(&parsed) {
                    transaction.commit().await?;
                    info!("LLM cache hit; skipped {} request", self.name());
                    return Ok(parsed);
                }
            }
            warn!("Discarding invalid cached LLM response");
            sqlx::query("DELETE FROM grammar_llm_cache WHERE request_sha256 = $1")
                .bind(key.as_slice())
                .execute(&mut *transaction)
                .await?;
        }

        info!("LLM cache miss; calling {}", self.name());
        let response = self.provider.generate(prompt, max_tokens).await?;
        let parsed = parse(&response).map_err(|error| self.unusable(error, &response))?;
        if reusable(&parsed) {
            sqlx::query(
                "INSERT INTO grammar_llm_cache (request_sha256, provider_identity, max_tokens, response)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(key.as_slice())
            .bind(identity)
            .bind(i64::from(max_tokens))
            .bind(&response)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(parsed)
    }

    /// An answer the caller cannot use is worth seeing: nothing else keeps it,
    /// and every rejection reads alike in an import log without it.
    fn unusable(&self, error: WisecrowError, response: &str) -> CachedLlmError {
        warn!(
            "{} answered unusably: {error}; answer began {}",
            self.name(),
            excerpt(response)
        );
        CachedLlmError::Response(error)
    }
}

/// Keeps a long or malformed answer from filling the import log.
fn excerpt(response: &str) -> &str {
    let end = response
        .char_indices()
        .nth(EXCERPT)
        .map_or(response.len(), |(index, _)| index);
    &response[..end]
}

fn request_key(
    provider: &str,
    identity: &str,
    prompt: &str,
    max_tokens: u32,
) -> Result<[u8; 32], WisecrowError> {
    digest(&(CACHE_VERSION, provider, identity, prompt, max_tokens))
}

pub(crate) fn digest(value: &impl serde::Serialize) -> Result<[u8; 32], WisecrowError> {
    let request = serde_json::to_vec(value)
        .map_err(|error| WisecrowError::ConfigurationError(error.to_string()))?;
    Ok(Sha256::digest(request).into())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    struct Model<'a> {
        name: &'a str,
        identity: Option<&'a str>,
        answer: Result<&'static str, &'static str>,
        calls: AtomicUsize,
    }

    impl Default for Model<'_> {
        fn default() -> Self {
            Self {
                name: "cache test",
                identity: Some("model-v1"),
                answer: Ok("42"),
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for Model<'_> {
        async fn generate(&self, _: &str, _: u32) -> Result<String, WisecrowError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            self.answer
                .map(str::to_owned)
                .map_err(|reason| WisecrowError::LlmError(reason.into()))
        }

        fn name(&self) -> &str {
            self.name
        }

        fn cache_identity(&self) -> Option<String> {
            self.identity.map(str::to_owned)
        }
    }

    fn parse(answer: &str) -> Result<i32, WisecrowError> {
        crate::llm::parse_fenced_json(answer, "cache fixture")
    }

    async fn generate(
        model: &CachedLlm<'_>,
        prompt: &str,
        tokens: u32,
    ) -> Result<i32, WisecrowError> {
        model
            .generate(prompt, tokens, parse, |n| *n >= 0)
            .await
            .map_err(Into::into)
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires PostgreSQL with database creation privileges"]
    async fn requests_persist_and_changes_require_new_calls(pool: PgPool) -> TestResult {
        let requests = [
            ("provider-a", "model-a:v1", "gd A1 à", 256),
            ("provider-b", "model-a:v1", "gd A1 à", 256),
            ("provider-a", "model-b:v1", "gd A1 à", 256),
            ("provider-a", "model-a:v2", "gd A1 à", 256),
            ("provider-a", "model-a:v1", "gd A2 à", 256),
            ("provider-a", "model-a:v1", "gd A1 ò", 256),
            ("provider-a", "model-a:v1", "gd A1 à", 512),
        ];
        for expected_calls in [1, 0] {
            for (name, identity, prompt, tokens) in requests {
                let provider = Model {
                    name,
                    identity: Some(identity),
                    ..Model::default()
                };
                let model = CachedLlm::new(&provider, Some(&pool));
                assert_eq!(generate(&model, prompt, tokens).await?, 42);
                assert_eq!(provider.calls.load(Ordering::SeqCst), expected_calls);
            }
        }
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires PostgreSQL with database creation privileges"]
    async fn failures_and_refusals_are_retryable(pool: PgPool) -> TestResult {
        for answer in [Err("quota exceeded"), Ok("invalid JSON"), Ok("-1")] {
            let provider = Model {
                answer,
                ..Model::default()
            };
            let model = CachedLlm::new(&provider, Some(&pool));
            let result = generate(&model, "retry", 256).await;
            match answer {
                Ok("-1") => assert_eq!(result?, -1),
                _ => assert!(result.is_err()),
            }
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM grammar_llm_cache")
                .fetch_one(&pool)
                .await?;
            assert_eq!(count, 0);
        }
        let repaired = Model::default();
        let model = CachedLlm::new(&repaired, Some(&pool));
        assert_eq!(generate(&model, "retry", 256).await?, 42);
        assert_eq!(generate(&model, "retry", 256).await?, 42);
        assert_eq!(repaired.calls.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires PostgreSQL with database creation privileges"]
    async fn concurrent_requests_share_one_generation_and_invalid_rows_are_repaired(
        pool: PgPool,
    ) -> TestResult {
        let provider = Model::default();
        let first = CachedLlm::new(&provider, Some(&pool));
        let second = CachedLlm::new(&provider, Some(&pool));
        let (left, right) = tokio::try_join!(
            generate(&first, "concurrent", 256),
            generate(&second, "concurrent", 256),
        )?;
        assert_eq!((left, right), (42, 42));
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);

        sqlx::query("UPDATE grammar_llm_cache SET response = 'invalid'")
            .execute(&pool)
            .await?;
        assert_eq!(generate(&first, "concurrent", 256).await?, 42);
        assert_eq!(generate(&second, "concurrent", 256).await?, 42);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
        Ok(())
    }

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires PostgreSQL with database creation privileges"]
    async fn cancellation_releases_the_request_lock_without_caching_a_result(
        pool: PgPool,
    ) -> TestResult {
        struct WaitingModel(tokio::sync::Notify);

        #[async_trait::async_trait]
        impl LlmProvider for WaitingModel {
            async fn generate(&self, _: &str, _: u32) -> Result<String, WisecrowError> {
                self.0.notify_one();
                std::future::pending().await
            }

            fn name(&self) -> &str {
                "cache test"
            }

            fn cache_identity(&self) -> Option<String> {
                Some("model-v1".into())
            }
        }

        let waiting = WaitingModel(tokio::sync::Notify::new());
        let model = CachedLlm::new(&waiting, Some(&pool));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            tokio::select! {
                result = generate(&model, "cancel", 256) => {
                    Err(format!("waiting model unexpectedly returned {result:?}"))
                }
                () = waiting.0.notified() => Ok(()),
            }
        })
        .await??;

        let resumed = Model::default();
        let model = CachedLlm::new(&resumed, Some(&pool));
        assert_eq!(
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                generate(&model, "cancel", 256)
            )
            .await??,
            42
        );
        assert_eq!(resumed.calls.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn calls_without_database_or_provider_identity_bypass_persistence() -> TestResult {
        let provider = Model::default();
        let model = CachedLlm::new(&provider, None);
        assert_eq!(generate(&model, "uncached", 256).await?, 42);
        assert_eq!(generate(&model, "uncached", 256).await?, 42);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);

        let pool = PgPoolOptions::new().connect_lazy("postgres://localhost/unavailable")?;
        let anonymous = Model {
            identity: None,
            ..Model::default()
        };
        let model = CachedLlm::new(&anonymous, Some(&pool));
        assert_eq!(generate(&model, "uncached", 256).await?, 42);
        assert_eq!(generate(&model, "uncached", 256).await?, 42);
        assert_eq!(anonymous.calls.load(Ordering::SeqCst), 2);
        Ok(())
    }
}
