use async_trait::async_trait;
use url::Url;

use crate::config::SecureString;
use crate::errors::WisecrowError;
use crate::media::image_provider::{ImageHit, ImageProvider, ImageQuery, Orientation};

const PROVIDER_ID: &str = "pexels";

pub struct PexelsProvider {
    api_key: SecureString,
}

impl PexelsProvider {
    #[must_use]
    pub fn new(api_key: SecureString) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl ImageProvider for PexelsProvider {
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
            Orientation::Squarish => "square",
            Orientation::Landscape => "landscape",
            Orientation::Portrait => "portrait",
        };

        let url = Url::parse_with_params(
            "https://api.pexels.com/v1/search",
            &[
                ("query", query.phrase),
                ("per_page", "1"),
                ("orientation", orientation),
            ],
        )?;

        let response = client
            .get(url)
            .header("Authorization", self.api_key.expose())
            .send()
            .await?
            .error_for_status()?;

        let json: serde_json::Value = response.json().await?;
        let photo = json["photos"].as_array().and_then(|photos| photos.first());
        let Some(photo) = photo else {
            return Ok(None);
        };

        let image_url = photo["src"]["medium"]
            .as_str()
            .or_else(|| photo["src"]["large"].as_str())
            .or_else(|| photo["src"]["original"].as_str());
        let Some(image_url) = image_url else {
            return Ok(None);
        };

        let download_url = Url::parse(image_url)
            .map_err(|e| WisecrowError::MediaError(format!("Invalid Pexels image URL: {e}")))?;

        let attribution = photo["photographer"]
            .as_str()
            .map(|name| format!("Photo by {name} on Pexels"));

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
        assert!(!PexelsProvider::new(SecureString::from(String::new())).is_configured());
    }
}
