use std::path::Path;
use std::sync::Arc;

use crate::config::{Config, SecureString};
use crate::errors::WisecrowError;
use crate::media::image_provider::{ImageProvider, ImageQuery};
use crate::media::providers::{PexelsProvider, PixabayProvider, UnsplashProvider};

const MAX_IMAGE_WIDTH: u32 = 200;
const MAX_IMAGE_HEIGHT: u32 = 200;

/// Which stock APIs to consult when building an [`ImageFetcher`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageProviderMode {
    /// Try every configured provider (Unsplash → Pexels → Pixabay).
    #[default]
    Auto,
    Unsplash,
    Pexels,
    Pixabay,
}

impl ImageProviderMode {
    /// Parses `auto` / `unsplash` / `pexels` / `pixabay` (case-insensitive).
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::ConfigurationError`] for unknown names.
    pub fn parse(raw: &str) -> Result<Self, WisecrowError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Ok(Self::Auto),
            "unsplash" => Ok(Self::Unsplash),
            "pexels" => Ok(Self::Pexels),
            "pixabay" => Ok(Self::Pixabay),
            other => Err(WisecrowError::ConfigurationError(format!(
                "Unknown image provider '{other}' (expected auto|unsplash|pexels|pixabay)"
            ))),
        }
    }
}

/// Ordered chain of stock-image providers with shared download + resize.
#[derive(Clone)]
pub struct ImageFetcher {
    providers: Vec<Arc<dyn ImageProvider>>,
}

impl ImageFetcher {
    #[must_use]
    pub fn new(providers: Vec<Arc<dyn ImageProvider>>) -> Self {
        Self {
            providers: providers
                .into_iter()
                .filter(|provider| provider.is_configured())
                .collect(),
        }
    }

    /// Builds a fetcher from config keys and optional mode override.
    ///
    /// Returns `None` when no API key is configured for the selected mode.
    #[must_use]
    pub fn from_config(config: &Config) -> Option<Self> {
        let mode = config
            .image_provider
            .as_deref()
            .and_then(|raw| ImageProviderMode::parse(raw).ok())
            .unwrap_or_default();
        Self::from_keys(
            mode,
            config.unsplash_api_key.as_ref(),
            config.pexels_api_key.as_ref(),
            config.pixabay_api_key.as_ref(),
        )
    }

    /// Builds a fetcher from individual keys (useful for CLI / tests).
    #[must_use]
    pub fn from_keys(
        mode: ImageProviderMode,
        unsplash: Option<&SecureString>,
        pexels: Option<&SecureString>,
        pixabay: Option<&SecureString>,
    ) -> Option<Self> {
        let mut providers: Vec<Arc<dyn ImageProvider>> = Vec::new();

        let push_unsplash = |providers: &mut Vec<Arc<dyn ImageProvider>>| {
            if let Some(key) = unsplash.filter(|k| !k.expose().trim().is_empty()) {
                // clone: SecureString ownership for provider storage
                providers.push(Arc::new(UnsplashProvider::new(key.clone())));
            }
        };
        let push_pexels = |providers: &mut Vec<Arc<dyn ImageProvider>>| {
            if let Some(key) = pexels.filter(|k| !k.expose().trim().is_empty()) {
                // clone: SecureString ownership for provider storage
                providers.push(Arc::new(PexelsProvider::new(key.clone())));
            }
        };
        let push_pixabay = |providers: &mut Vec<Arc<dyn ImageProvider>>| {
            if let Some(key) = pixabay.filter(|k| !k.expose().trim().is_empty()) {
                // clone: SecureString ownership for provider storage
                providers.push(Arc::new(PixabayProvider::new(key.clone())));
            }
        };

        match mode {
            ImageProviderMode::Auto => {
                push_unsplash(&mut providers);
                push_pexels(&mut providers);
                push_pixabay(&mut providers);
            }
            ImageProviderMode::Unsplash => push_unsplash(&mut providers),
            ImageProviderMode::Pexels => push_pexels(&mut providers),
            ImageProviderMode::Pixabay => push_pixabay(&mut providers),
        }

        let fetcher = Self::new(providers);
        if fetcher.is_empty() {
            None
        } else {
            Some(fetcher)
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    #[must_use]
    pub fn provider_ids(&self) -> Vec<&'static str> {
        self.providers.iter().map(|p| p.id()).collect()
    }

    /// Search providers in order, download the first hit, resize to card size.
    ///
    /// # Errors
    ///
    /// Returns an error when no provider yields a usable image.
    pub async fn fetch_bytes(
        &self,
        client: &reqwest::Client,
        query: &ImageQuery<'_>,
    ) -> Result<Vec<u8>, WisecrowError> {
        if self.providers.is_empty() {
            return Err(WisecrowError::MediaError(
                "No image provider is configured".to_owned(),
            ));
        }

        let mut last_error: Option<WisecrowError> = None;
        for provider in &self.providers {
            match provider.search(client, query).await {
                Ok(Some(hit)) => match download_and_resize(client, &hit.download_url).await {
                    Ok(bytes) => {
                        if let Some(ref attribution) = hit.attribution {
                            tracing::debug!(
                                provider = hit.provider_id,
                                %attribution,
                                "Fetched card image"
                            );
                        }
                        return Ok(bytes);
                    }
                    Err(error) => {
                        tracing::warn!(
                            provider = provider.id(),
                            ?error,
                            "Image download failed; trying next provider"
                        );
                        last_error = Some(error);
                    }
                },
                Ok(None) => {
                    tracing::debug!(provider = provider.id(), "No image hit for query");
                }
                Err(error) => {
                    tracing::warn!(
                        provider = provider.id(),
                        ?error,
                        "Image search failed; trying next provider"
                    );
                    last_error = Some(error);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            WisecrowError::MediaError("No image provider returned a usable hit".to_owned())
        }))
    }
}

async fn download_and_resize(
    client: &reqwest::Client,
    image_url: &url::Url,
) -> Result<Vec<u8>, WisecrowError> {
    let image_bytes = client
        .get(image_url.clone()) // clone: Url owned by reqwest RequestBuilder
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    resize_image(&image_bytes)
}

/// Fetches an image for a vocabulary word via the configured provider chain.
///
/// Prefer [`ImageFetcher::fetch_bytes`] when a fetcher is already available.
///
/// # Errors
///
/// Returns an error if no provider is configured or all providers fail.
pub async fn fetch_image(
    client: &reqwest::Client,
    word: &str,
    fetcher: &ImageFetcher,
) -> Result<Vec<u8>, WisecrowError> {
    fetcher.fetch_bytes(client, &ImageQuery::new(word)).await
}

/// Resizes image bytes to fit within the maximum dimensions while
/// preserving aspect ratio.
///
/// # Errors
///
/// Returns an error if the image cannot be decoded.
pub fn resize_image(data: &[u8]) -> Result<Vec<u8>, WisecrowError> {
    let img = image::load_from_memory(data)
        .map_err(|e| WisecrowError::MediaError(format!("Failed to decode image: {e}")))?;

    let resized = img.resize(
        MAX_IMAGE_WIDTH,
        MAX_IMAGE_HEIGHT,
        image::imageops::FilterType::Lanczos3,
    );

    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    resized
        .write_to(&mut cursor, image::ImageFormat::Jpeg)
        .map_err(|e| WisecrowError::MediaError(format!("Failed to encode image: {e}")))?;

    Ok(buf)
}

/// Loads a cached image and returns a ratatui-image stateful protocol
/// for rendering in the TUI.
///
/// # Errors
///
/// Returns an error if the image file cannot be read, decoded, or the
/// terminal does not support image protocols.
pub fn load_image_for_display(
    path: &Path,
) -> Result<Box<dyn ratatui_image::protocol::StatefulProtocol>, WisecrowError> {
    let dyn_img = image::ImageReader::open(path)
        .map_err(|e| WisecrowError::MediaError(format!("Failed to open image: {e}")))?
        .decode()
        .map_err(|e| WisecrowError::MediaError(format!("Failed to decode image: {e}")))?;

    let mut picker = ratatui_image::picker::Picker::from_termios()
        .map_err(|e| WisecrowError::MediaError(format!("Terminal does not support images: {e}")))?;

    Ok(picker.new_resize_protocol(dyn_img))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::image_provider::{ImageHit, Orientation};
    use async_trait::async_trait;
    use proptest::prelude::*;
    use url::Url;

    struct MockProvider {
        id: &'static str,
        hit: Option<ImageHit>,
        fail: bool,
    }

    #[async_trait]
    impl ImageProvider for MockProvider {
        fn id(&self) -> &'static str {
            self.id
        }

        fn is_configured(&self) -> bool {
            true
        }

        async fn search(
            &self,
            _client: &reqwest::Client,
            _query: &ImageQuery<'_>,
        ) -> Result<Option<ImageHit>, WisecrowError> {
            if self.fail {
                return Err(WisecrowError::MediaError("mock search failed".to_owned()));
            }
            // clone: return owned hit from mock
            Ok(self.hit.clone())
        }
    }

    #[test]
    fn resize_preserves_valid_image() {
        let img = image::DynamicImage::new_rgb8(400, 300);
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        img.write_to(&mut cursor, image::ImageFormat::Png).unwrap();

        let resized = resize_image(&buf).unwrap();
        let decoded = image::load_from_memory(&resized).unwrap();
        assert!(decoded.width() <= MAX_IMAGE_WIDTH);
        assert!(decoded.height() <= MAX_IMAGE_HEIGHT);
    }

    #[test]
    fn resize_rejects_invalid_data() {
        let result = resize_image(b"not an image");
        assert!(result.is_err());
    }

    proptest! {
        #[test]
        fn resize_image_never_panics(data in proptest::collection::vec(any::<u8>(), 0..1000)) {
            let _ = resize_image(&data);
        }
    }

    #[test]
    fn provider_mode_parse() {
        assert_eq!(
            ImageProviderMode::parse("auto").unwrap(),
            ImageProviderMode::Auto
        );
        assert_eq!(
            ImageProviderMode::parse("PEXELS").unwrap(),
            ImageProviderMode::Pexels
        );
        assert!(ImageProviderMode::parse("flickr").is_err());
    }

    #[test]
    fn from_keys_auto_skips_empty() {
        let fetcher = ImageFetcher::from_keys(
            ImageProviderMode::Auto,
            Some(&SecureString::from("u".to_owned())),
            None,
            Some(&SecureString::from("  ".to_owned())),
        )
        .expect("unsplash key present");
        assert_eq!(fetcher.provider_ids(), vec!["unsplash"]);
    }

    #[test]
    fn from_keys_mode_filters() {
        let fetcher = ImageFetcher::from_keys(
            ImageProviderMode::Pexels,
            Some(&SecureString::from("u".to_owned())),
            Some(&SecureString::from("p".to_owned())),
            Some(&SecureString::from("x".to_owned())),
        )
        .expect("pexels key present");
        assert_eq!(fetcher.provider_ids(), vec!["pexels"]);
    }

    #[test]
    fn from_keys_none_when_empty() {
        assert!(ImageFetcher::from_keys(ImageProviderMode::Auto, None, None, None).is_none());
    }

    #[tokio::test]
    async fn fetcher_returns_error_when_all_fail() {
        let fetcher = ImageFetcher::new(vec![
            Arc::new(MockProvider {
                id: "a",
                hit: None,
                fail: false,
            }),
            Arc::new(MockProvider {
                id: "b",
                hit: None,
                fail: true,
            }),
        ]);
        let client = reqwest::Client::new();
        let err = fetcher
            .fetch_bytes(&client, &ImageQuery::new("cat"))
            .await
            .unwrap_err();
        assert!(matches!(err, WisecrowError::MediaError(_)));
    }

    #[test]
    fn image_query_defaults_squarish() {
        let q = ImageQuery::new("house");
        assert_eq!(q.orientation, Orientation::Squarish);
        assert!(q.lang_hint.is_none());
    }

    #[test]
    fn mock_hit_clone_roundtrip() {
        let hit = ImageHit {
            download_url: Url::parse("https://example.com/a.jpg").unwrap(),
            attribution: Some("test".to_owned()),
            provider_id: "mock",
        };
        // clone: test ownership independence
        let other = hit.clone();
        assert_eq!(other.provider_id, "mock");
    }
}
