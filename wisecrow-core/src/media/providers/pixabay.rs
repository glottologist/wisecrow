use async_trait::async_trait;
use url::Url;

use crate::config::SecureString;
use crate::errors::WisecrowError;
use crate::media::image_provider::{ImageHit, ImageProvider, ImageQuery, Orientation};

const PROVIDER_ID: &str = "pixabay";

pub struct PixabayProvider {
    api_key: SecureString,
}

impl PixabayProvider {
    #[must_use]
    pub fn new(api_key: SecureString) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl ImageProvider for PixabayProvider {
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

        // Pixabay supports horizontal/vertical only; map squarish → all.
        let orientation = match query.orientation {
            Orientation::Landscape => "horizontal",
            Orientation::Portrait => "vertical",
            Orientation::Squarish => "all",
        };

        let url = Url::parse_with_params(
            "https://pixabay.com/api/",
            &[
                ("key", self.api_key.expose()),
                ("q", query.phrase),
                ("image_type", "photo"),
                ("orientation", orientation),
                ("per_page", "3"),
                ("safesearch", "true"),
            ],
        )?;

        let response = client.get(url).send().await?.error_for_status()?;
        let json: serde_json::Value = response.json().await?;
        let hit = json["hits"].as_array().and_then(|hits| hits.first());
        let Some(hit) = hit else {
            return Ok(None);
        };

        let image_url = hit["webformatURL"]
            .as_str()
            .or_else(|| hit["largeImageURL"].as_str())
            .or_else(|| hit["previewURL"].as_str());
        let Some(image_url) = image_url else {
            return Ok(None);
        };

        let download_url = Url::parse(image_url)
            .map_err(|e| WisecrowError::MediaError(format!("Invalid Pixabay image URL: {e}")))?;

        let attribution = hit["user"]
            .as_str()
            .map(|name| format!("Image by {name} on Pixabay"));

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
        assert!(!PixabayProvider::new(SecureString::from(String::new())).is_configured());
    }
}
