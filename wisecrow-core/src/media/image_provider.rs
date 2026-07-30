//! Stock-photo provider abstraction for vocabulary card images.

use async_trait::async_trait;
use url::Url;

use crate::errors::WisecrowError;

/// Preferred crop for card-sized display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Squarish,
    Landscape,
    Portrait,
}

/// Search request shared by all stock providers.
#[derive(Debug, Clone, Copy)]
pub struct ImageQuery<'a> {
    pub phrase: &'a str,
    pub orientation: Orientation,
    pub lang_hint: Option<&'a str>,
}

impl<'a> ImageQuery<'a> {
    #[must_use]
    pub fn new(phrase: &'a str) -> Self {
        Self {
            phrase,
            orientation: Orientation::Squarish,
            lang_hint: None,
        }
    }
}

/// One normalised search hit before the image bytes are downloaded.
#[derive(Debug, Clone)]
pub struct ImageHit {
    pub download_url: Url,
    pub attribution: Option<String>,
    pub provider_id: &'static str,
}

/// Keyword → stock photo search backend.
#[async_trait]
pub trait ImageProvider: Send + Sync {
    fn id(&self) -> &'static str;

    fn is_configured(&self) -> bool;

    /// Returns the best hit for `query`, or `None` when the API has no match.
    async fn search(
        &self,
        client: &reqwest::Client,
        query: &ImageQuery<'_>,
    ) -> Result<Option<ImageHit>, WisecrowError>;
}
