//! Stable source fingerprints for generated media.

use std::fmt;

use sha2::{Digest, Sha256};

const ROOT_DOMAIN: &str = "wisecrow-media-fingerprint-v1";
const AUDIO_DOMAIN: &str = "audio";
const IMAGE_DOMAIN: &str = "image";

/// SHA-256 identity of every input that determines generated media bytes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MediaFingerprint(String);

impl MediaFingerprint {
    /// Fingerprints canonical speech text and the selected synthesis profile.
    #[must_use]
    pub fn for_audio(
        text: &str,
        language: &str,
        backend: &str,
        voice: &str,
        format_version: u32,
    ) -> Self {
        let mut fingerprint = FingerprintBuilder::new(AUDIO_DOMAIN);
        fingerprint.add_text("text", text);
        fingerprint.add_text("language", language);
        fingerprint.add_text("backend", backend);
        fingerprint.add_text("voice", voice);
        fingerprint.add_bytes("format-version", &format_version.to_be_bytes());
        fingerprint.finish()
    }

    /// Fingerprints a canonical image query and ordered provider chain.
    #[must_use]
    pub fn for_image(
        query: &str,
        language: &str,
        provider_ids: &[&str],
        format_version: u32,
    ) -> Self {
        let mut fingerprint = FingerprintBuilder::new(IMAGE_DOMAIN);
        fingerprint.add_text("query", query);
        fingerprint.add_text("language", language);
        for provider_id in provider_ids {
            fingerprint.add_text("provider", provider_id);
        }
        fingerprint.add_bytes("format-version", &format_version.to_be_bytes());
        fingerprint.finish()
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for MediaFingerprint {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for MediaFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

struct FingerprintBuilder(Sha256);

impl FingerprintBuilder {
    fn new(domain: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(Sha256::digest(ROOT_DOMAIN.as_bytes()));
        hasher.update(Sha256::digest(domain.as_bytes()));
        Self(hasher)
    }

    fn add_text(&mut self, label: &str, value: &str) {
        self.add_bytes(label, value.as_bytes());
    }

    fn add_bytes(&mut self, label: &str, value: &[u8]) {
        self.0.update(Sha256::digest(label.as_bytes()));
        self.0.update(Sha256::digest(value));
    }

    fn finish(self) -> MediaFingerprint {
        MediaFingerprint(format!("{:x}", self.0.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn audio_fingerprint_covers_every_output_input(text in any::<String>(), language in any::<String>()) {
            let baseline = MediaFingerprint::for_audio(&text, &language, "edge", "voice-a", 1);
            let identical = MediaFingerprint::for_audio(&text, &language, "edge", "voice-a", 1);

            prop_assert_eq!(&baseline, &identical);
            prop_assert_ne!(&baseline, &MediaFingerprint::for_audio(
                &format!("{text}\0"), &language, "edge", "voice-a", 1
            ));
            prop_assert_ne!(&baseline, &MediaFingerprint::for_audio(
                &text, &format!("{language}\0"), "edge", "voice-a", 1
            ));
            prop_assert_ne!(&baseline, &MediaFingerprint::for_audio(
                &text, &language, "cereproc", "voice-a", 1
            ));
            prop_assert_ne!(&baseline, &MediaFingerprint::for_audio(
                &text, &language, "edge", "voice-b", 1
            ));
            prop_assert_ne!(&baseline, &MediaFingerprint::for_audio(
                &text, &language, "edge", "voice-a", 2
            ));
        }

        #[test]
        fn image_fingerprint_covers_every_output_input(query in any::<String>(), language in any::<String>()) {
            let providers = ["unsplash", "pexels"];
            let reversed = ["pexels", "unsplash"];
            let baseline = MediaFingerprint::for_image(&query, &language, &providers, 1);
            let identical = MediaFingerprint::for_image(&query, &language, &providers, 1);

            prop_assert_eq!(&baseline, &identical);
            prop_assert_ne!(&baseline, &MediaFingerprint::for_image(
                &format!("{query}\0"), &language, &providers, 1
            ));
            prop_assert_ne!(&baseline, &MediaFingerprint::for_image(
                &query, &format!("{language}\0"), &providers, 1
            ));
            prop_assert_ne!(&baseline, &MediaFingerprint::for_image(
                &query, &language, &reversed, 1
            ));
            prop_assert_ne!(&baseline, &MediaFingerprint::for_image(
                &query, &language, &providers, 2
            ));
        }
    }
}
