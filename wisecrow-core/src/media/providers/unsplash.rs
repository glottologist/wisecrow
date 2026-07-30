use async_trait::async_trait;
use url::Url;

use crate::config::SecureString;
use crate::errors::WisecrowError;
use crate::media::image_provider::{ImageHit, ImageProvider, ImageQuery, Orientation};

const PROVIDER_ID: &str = "unsplash";

pub struct UnsplashProvider {
    api_key: SecureString,
}

impl UnsplashProvider {
    #[must_use]
    pub fn new(api_key: SecureString) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl ImageProvider for UnsplashProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn is_configured(&self) -> bool {
        !self.api_key.expose().trim().is_empty()
    }

    async fn search(
        &self,
        client: &reqwest::Client,
        query: &ImageQuery<'_>,
    ) -> Result<Option<ImageHit>, WisecrowError> {
        if query.phrase.trim().is_empty() {
            return Ok(None);
        }

        let orientation = match query.orientation {
            Orientation::Squarish => "squarish",
            Orientation::Landscape => "landscape",
            Orientation::Portrait => "portrait",
        };

        let url = Url::parse_with_params(
            "https://api.unsplash.com/photos/random",
            &[("query", query.phrase), ("orientation", orientation)],
        )?;

        let response = client
            .get(url)
            .header(
                "Authorization",
                format!("Client-ID {}", self.api_key.expose()),
            )
            .send()
            .await?
            .error_for_status()?;

        let json: serde_json::Value = response.json().await?;
        let Some(image_url) = json["urls"]["small"].as_str() else {
            return Ok(None);
        };
        let download_url = Url::parse(image_url)
            .map_err(|e| WisecrowError::MediaError(format!("Invalid Unsplash image URL: {e}")))?;

        let attribution = json["user"]["name"]
            .as_str()
            .map(|name| format!("Photo by {name} on Unsplash"));

        Ok(Some(ImageHit {
            download_url,
            attribution,
            provider_id: PROVIDER_ID,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_key_is_not_configured() {
        let provider = UnsplashProvider::new(SecureString::from(String::new()));
        assert!(!provider.is_configured());
    }

    #[test]
    fn non_empty_key_is_configured() {
        let provider = UnsplashProvider::new(SecureString::from("test-key".to_owned()));
        assert!(provider.is_configured());
    }
}
