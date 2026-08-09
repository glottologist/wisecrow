//! Speech synthesis through the CereVoice Cloud.
//!
//! MS Edge covers 83 languages but no Scottish Gaelic, and its live voice list
//! confirms the gap rather than merely omitting it: 322 voices across 74
//! languages, Welsh and Irish among them, Gaelic absent. CereProc is the only
//! service found to carry a Gaelic voice, and it carries Irish and Welsh too —
//! the latter split by dialect, which nothing else offers.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::config::{Config, SecureString};
use crate::errors::WisecrowError;

const API_BASE: &str = "https://api.cerevoice.com/v2";

/// Access tokens last three hours. Renewing early keeps a synthesis that
/// starts near the boundary from arriving after expiry.
const TOKEN_LIFETIME: Duration = Duration::from_secs(2 * 60 * 60 + 45 * 60);

/// North-Wales Ffion. The eight Welsh voices split north (`-NW-`) and south
/// (`-SW-`); this deployment learns northern Welsh, and `cereproc_welsh_voice`
/// selects any of the others.
const DEFAULT_WELSH_VOICE: &str = "Ffion-CY-NW-T-F";

/// Languages served by CereProc rather than Edge. Welsh is absent here
/// because its voice is configurable — see [`CereprocClient::voice_for_language`].
const VOICES: &[(&str, &str)] = &[("gd", "Ceitidh"), ("ga", "Peig")];

#[derive(Deserialize)]
struct AuthResponse {
    access_token: String,
}

struct CachedToken {
    value: String,
    issued_at: Instant,
}

/// Client for the CereVoice Cloud, holding one shared access token.
///
/// Cloning shares that token, so a fleet of prefetch tasks authenticates once
/// between them rather than once each.
#[derive(Clone)]
pub struct CereprocClient {
    http: reqwest::Client,
    email: String,
    password: SecureString,
    welsh_voice: String,
    token: Arc<Mutex<Option<CachedToken>>>,
}

impl CereprocClient {
    /// Builds a client from configuration.
    ///
    /// Returns `None` unless both account fields carry a non-blank value, so an
    /// unconfigured deployment falls through to Edge instead of failing.
    #[must_use]
    pub fn from_config(config: &Config) -> Option<Self> {
        let email = config
            .cereproc_email
            .as_deref()
            .map(str::trim)
            .filter(|email| !email.is_empty())?;
        let password = config
            .cereproc_password
            .as_ref()
            .filter(|password| !password.expose().trim().is_empty())?;
        let welsh_voice = config
            .cereproc_welsh_voice
            .as_deref()
            .map(str::trim)
            .filter(|voice| !voice.is_empty())
            .unwrap_or(DEFAULT_WELSH_VOICE);

        Some(Self {
            http: reqwest::Client::new(),
            email: String::from(email),
            password: password.clone(), // clone: SecureString ownership for client storage
            welsh_voice: String::from(welsh_voice),
            token: Arc::new(Mutex::new(None)),
        })
    }

    /// Returns the CereProc voice for a language, or `None` when Edge should
    /// handle it.
    #[must_use]
    pub fn voice_for_language(&self, lang_code: &str) -> Option<&str> {
        if lang_code == "cy" {
            return Some(&self.welsh_voice);
        }
        VOICES
            .iter()
            .find_map(|(code, voice)| (*code == lang_code).then_some(*voice))
    }

    /// Synthesises `text` as MP3 with the named voice.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::MediaError`] if authentication is rejected, the
    /// synthesis request fails, or the service returns an error status.
    pub async fn synthesise(&self, text: &str, voice: &str) -> Result<Vec<u8>, WisecrowError> {
        let response = self
            .speak(text, voice, &self.access_token(false).await?)
            .await?;

        // A cached token can be revoked or invalidated before its local
        // lifetime elapses; one forced re-auth distinguishes that from a
        // genuine rejection of the account.
        let response = if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.speak(text, voice, &self.access_token(true).await?)
                .await?
        } else {
            response
        };

        let status = response.status();
        if !status.is_success() {
            return Err(WisecrowError::MediaError(format!(
                "CereProc synthesis failed for voice {voice}: HTTP {status}"
            )));
        }

        let audio = response
            .bytes()
            .await
            .map_err(|error| WisecrowError::MediaError(format!("CereProc audio read: {error}")))?;
        if audio.is_empty() {
            return Err(WisecrowError::MediaError(format!(
                "CereProc returned no audio for voice {voice}"
            )));
        }
        Ok(audio.to_vec())
    }

    async fn speak(
        &self,
        text: &str,
        voice: &str,
        token: &str,
    ) -> Result<reqwest::Response, WisecrowError> {
        // Plain text rather than XML: card phrases are user-facing strings that
        // would otherwise need escaping, and `<` in a phrase would silently
        // become markup.
        self.http
            .post(format!("{API_BASE}/speak"))
            .query(&[("voice", voice), ("audio_format", "mp3")])
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "text/plain")
            .body(String::from(text))
            .send()
            .await
            .map_err(|error| WisecrowError::MediaError(format!("CereProc request failed: {error}")))
    }

    async fn access_token(&self, force_refresh: bool) -> Result<String, WisecrowError> {
        if !force_refresh {
            if let Some(cached) = self.cached_token() {
                return Ok(cached);
            }
        }
        let token = self.authenticate().await?;
        if let Ok(mut guard) = self.token.lock() {
            *guard = Some(CachedToken {
                value: token.clone(), // clone: cache keeps its own copy
                issued_at: Instant::now(),
            });
        }
        Ok(token)
    }

    /// A poisoned lock yields `None` rather than propagating: the only cost of
    /// treating the cache as empty is one extra authentication.
    fn cached_token(&self) -> Option<String> {
        let guard = self.token.lock().ok()?;
        let cached = guard.as_ref()?;
        (cached.issued_at.elapsed() < TOKEN_LIFETIME).then(|| cached.value.clone())
    }

    async fn authenticate(&self) -> Result<String, WisecrowError> {
        let response = self
            .http
            .get(format!("{API_BASE}/auth"))
            .basic_auth(&self.email, Some(self.password.expose()))
            .send()
            .await
            .map_err(|error| {
                WisecrowError::MediaError(format!("CereProc authentication failed: {error}"))
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(WisecrowError::MediaError(format!(
                "CereProc rejected the account credentials: HTTP {status}"
            )));
        }

        response
            .json::<AuthResponse>()
            .await
            .map(|parsed| parsed.access_token)
            .map_err(|error| {
                WisecrowError::MediaError(format!("CereProc authentication response: {error}"))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(email: Option<&str>, password: Option<&str>, welsh: Option<&str>) -> Config {
        Config {
            db_url: None,
            db_address: None,
            db_name: None,
            db_user: None,
            db_password: None,
            image_provider: None,
            unsplash_api_key: None,
            pexels_api_key: None,
            pixabay_api_key: None,
            cereproc_email: email.map(String::from),
            cereproc_password: password.map(|value| SecureString::from(String::from(value))),
            cereproc_welsh_voice: welsh.map(String::from),
            llm_provider: None,
            llm_api_key: None,
            llm_model: None,
            remote_url: None,
            remote_api_key: None,
            sync_api_key: None,
        }
    }

    #[test]
    fn unconfigured_account_yields_no_client() {
        assert!(CereprocClient::from_config(&config_with(None, None, None)).is_none());
        assert!(CereprocClient::from_config(&config_with(Some("a@b.c"), None, None)).is_none());
        assert!(CereprocClient::from_config(&config_with(None, Some("secret"), None)).is_none());
    }

    #[test]
    fn blank_credentials_yield_no_client() {
        assert!(
            CereprocClient::from_config(&config_with(Some("   "), Some("secret"), None)).is_none()
        );
        assert!(
            CereprocClient::from_config(&config_with(Some("a@b.c"), Some("  "), None)).is_none()
        );
    }

    #[test]
    fn celtic_languages_map_to_voices() {
        let client = CereprocClient::from_config(&config_with(Some("a@b.c"), Some("secret"), None))
            .expect("both credentials present");
        assert_eq!(client.voice_for_language("gd"), Some("Ceitidh"));
        assert_eq!(client.voice_for_language("ga"), Some("Peig"));
        assert_eq!(client.voice_for_language("cy"), Some(DEFAULT_WELSH_VOICE));
    }

    #[test]
    fn other_languages_fall_through_to_edge() {
        let client = CereprocClient::from_config(&config_with(Some("a@b.c"), Some("secret"), None))
            .expect("both credentials present");
        for lang in ["en", "fr", "de", "cs"] {
            assert_eq!(
                client.voice_for_language(lang),
                None,
                "{lang} belongs to Edge"
            );
        }
    }

    #[test]
    fn welsh_voice_is_configurable() {
        // A southern voice, so the assertion cannot pass by matching the default.
        let client = CereprocClient::from_config(&config_with(
            Some("a@b.c"),
            Some("secret"),
            Some("Gethin-CY-SW-C-M"),
        ))
        .expect("both credentials present");
        assert_eq!(client.voice_for_language("cy"), Some("Gethin-CY-SW-C-M"));
    }

    #[test]
    fn blank_welsh_override_falls_back_to_default() {
        let client =
            CereprocClient::from_config(&config_with(Some("a@b.c"), Some("secret"), Some("  ")))
                .expect("both credentials present");
        assert_eq!(client.voice_for_language("cy"), Some(DEFAULT_WELSH_VOICE));
    }
}
