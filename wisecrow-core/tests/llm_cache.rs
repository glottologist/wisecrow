//! The model-response cache is reachable outside the grammar importer.

use std::sync::atomic::{AtomicUsize, Ordering};

use wisecrow::errors::WisecrowError;
use wisecrow::llm::cache::CachedLlm;
use wisecrow::llm::LlmProvider;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Counting(AtomicUsize);

#[async_trait::async_trait]
impl LlmProvider for Counting {
    async fn generate(&self, _prompt: &str, _max_tokens: u32) -> Result<String, WisecrowError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok("7".to_owned())
    }

    fn name(&self) -> &str {
        "counting"
    }
}

#[tokio::test]
async fn a_provider_without_a_cache_identity_is_asked_every_time() -> TestResult {
    let provider = Counting(AtomicUsize::new(0));
    let model = CachedLlm::new(&provider, None);
    for _ in 0..2 {
        let answer: i32 = model
            .generate(
                "uncached",
                16,
                |response| wisecrow::llm::parse_fenced_json(response, "a number"),
                |_| true,
            )
            .await?;
        assert_eq!(answer, 7);
    }
    assert_eq!(provider.0.load(Ordering::SeqCst), 2);
    Ok(())
}
