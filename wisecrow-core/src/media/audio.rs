#[cfg(feature = "audio")]
use std::path::Path;

use crate::errors::WisecrowError;

const TTS_VOICES: &[(&str, &str)] = &[
    ("af", "af-ZA-AdriNeural"),
    ("am", "am-ET-AmehaNeural"),
    ("ar", "ar-SA-HamedNeural"),
    ("bg", "bg-BG-BorislavNeural"),
    ("bn", "bn-IN-BashkarNeural"),
    ("bs", "bs-BA-GoranNeural"),
    ("ca", "ca-ES-EnricNeural"),
    ("cs", "cs-CZ-AntoninNeural"),
    ("cy", "cy-GB-AledNeural"),
    ("da", "da-DK-JeppeNeural"),
    ("de", "de-DE-ConradNeural"),
    ("el", "el-GR-NestorasNeural"),
    ("en", "en-US-GuyNeural"),
    ("es", "es-ES-AlvaroNeural"),
    ("et", "et-EE-KertNeural"),
    ("fa", "fa-IR-FaridNeural"),
    ("fi", "fi-FI-HarriNeural"),
    ("fr", "fr-FR-HenriNeural"),
    ("ga", "ga-IE-ColmNeural"),
    ("gl", "gl-ES-RoiNeural"),
    ("gu", "gu-IN-NiranjanNeural"),
    ("he", "he-IL-AvriNeural"),
    ("hi", "hi-IN-MadhurNeural"),
    ("hr", "hr-HR-SreckoNeural"),
    ("hu", "hu-HU-TamasNeural"),
    ("id", "id-ID-ArdiNeural"),
    ("is", "is-IS-GunnarNeural"),
    ("it", "it-IT-DiegoNeural"),
    ("ja", "ja-JP-KeitaNeural"),
    ("jv", "jv-ID-DimasNeural"),
    ("ka", "ka-GE-GiorgiNeural"),
    ("kk", "kk-KZ-DauletNeural"),
    ("km", "km-KH-PisethNeural"),
    ("kn", "kn-IN-GaganNeural"),
    ("ko", "ko-KR-InJoonNeural"),
    ("lo", "lo-LA-ChanthavongNeural"),
    ("lt", "lt-LT-LeonasNeural"),
    ("lv", "lv-LV-NilsNeural"),
    ("mk", "mk-MK-AleksandarNeural"),
    ("ml", "ml-IN-MidhunNeural"),
    ("mn", "mn-MN-BataaNeural"),
    ("mr", "mr-IN-ManoharNeural"),
    ("ms", "ms-MY-OsmanNeural"),
    ("my", "my-MM-ThihaNeural"),
    ("ne", "ne-NP-SagarNeural"),
    ("nl", "nl-NL-MaartenNeural"),
    ("no", "nb-NO-FinnNeural"),
    ("pa", "pa-IN-GurdeepNeural"),
    ("pl", "pl-PL-MarekNeural"),
    ("ps", "ps-AF-GulNawazNeural"),
    ("pt", "pt-BR-AntonioNeural"),
    ("ro", "ro-RO-EmilNeural"),
    ("ru", "ru-RU-DmitryNeural"),
    ("si", "si-LK-SameeraNeural"),
    ("sk", "sk-SK-LukasNeural"),
    ("sl", "sl-SI-RokNeural"),
    ("so", "so-SO-MuuseNeural"),
    ("sq", "sq-AL-IlirNeural"),
    ("sr", "sr-RS-NicholasNeural"),
    ("su", "su-ID-JajangNeural"),
    ("sv", "sv-SE-MattiasNeural"),
    ("sw", "sw-KE-RafikiNeural"),
    ("ta", "ta-IN-ValluvarNeural"),
    ("te", "te-IN-MohanNeural"),
    ("th", "th-TH-NiwatNeural"),
    ("tl", "fil-PH-BlessicaNeural"),
    ("tr", "tr-TR-AhmetNeural"),
    ("uk", "uk-UA-OstapNeural"),
    ("ur", "ur-PK-AsadNeural"),
    ("uz", "uz-UZ-SardorNeural"),
    ("vi", "vi-VN-NamMinhNeural"),
    ("zh", "zh-CN-YunxiNeural"),
    ("zu", "zu-ZA-ThembaNeural"),
];

/// Maps wisecrow language codes to MS Edge TTS voice names.
#[must_use]
pub fn voice_for_language(lang_code: &str) -> Option<&'static str> {
    TTS_VOICES
        .iter()
        .find_map(|(code, voice)| (*code == lang_code).then_some(*voice))
}

/// Generates MP3 audio for the given text using MS Edge TTS.
///
/// # Errors
///
/// Returns an error if the TTS service is unavailable or the language
/// is not supported.
pub async fn generate_tts(text: &str, lang_code: &str) -> Result<Vec<u8>, WisecrowError> {
    let voice = voice_for_language(lang_code).ok_or_else(|| {
        WisecrowError::MediaError(format!("No TTS voice available for language: {lang_code}"))
    })?;

    let text = String::from(text);
    let voice = String::from(voice);

    tokio::task::spawn_blocking(move || {
        let mut tts = msedge_tts::tts::client::connect()
            .map_err(|e| WisecrowError::MediaError(format!("TTS connection failed: {e}")))?;

        let config = msedge_tts::tts::SpeechConfig::from(
            &msedge_tts::voice::get_voices_list()
                .map_err(|e| WisecrowError::MediaError(format!("Failed to get voices: {e}")))?
                .into_iter()
                .find(|candidate| candidate.short_name.as_deref() == Some(voice.as_str()))
                .ok_or_else(|| WisecrowError::MediaError(format!("Voice not found: {voice}")))?,
        );

        let audio = tts
            .synthesize(&text, &config)
            .map_err(|e| WisecrowError::MediaError(format!("TTS synthesis failed: {e}")))?;

        Ok(audio.audio_bytes)
    })
    .await
    .map_err(|e| WisecrowError::MediaError(format!("TTS task panicked: {e}")))?
}

/// Plays an audio file from the given path. Non-blocking (spawns a thread).
///
/// # Errors
///
/// Returns an error if the audio file cannot be opened or the output
/// device is unavailable.
#[cfg(feature = "audio")]
pub fn play_audio(path: &Path) -> Result<(), WisecrowError> {
    use std::fs::File;
    use std::io::BufReader;

    let file = File::open(path)?;
    let reader = BufReader::new(file);

    std::thread::spawn(move || {
        let Ok((_stream, handle)) = rodio::OutputStream::try_default() else {
            tracing::debug!("No audio output device available");
            return;
        };
        let Ok(sink) = rodio::Sink::try_new(&handle) else {
            tracing::debug!("Failed to create audio sink");
            return;
        };
        match rodio::Decoder::new(reader) {
            Ok(source) => {
                sink.append(source);
                sink.sleep_until_end();
            }
            Err(e) => tracing::debug!("Failed to decode audio: {e}"),
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::SUPPORTED_LANGUAGE_INFO;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn unknown_code_returns_none(s in "[a-z0-9]{1,10}") {
            let is_known = SUPPORTED_LANGUAGE_INFO.iter().any(|(c, _)| *c == s);
            if !is_known {
                prop_assert!(voice_for_language(&s).is_none());
            }
        }
    }

    #[test]
    fn all_voices_are_neural() {
        let mut count = 0;
        for (code, _) in SUPPORTED_LANGUAGE_INFO {
            if let Some(voice) = voice_for_language(code) {
                assert!(
                    voice.contains("Neural"),
                    "Voice for {code} is not Neural: {voice}"
                );
                count += 1;
            }
        }
        assert!(
            count >= 10,
            "Expected at least 10 languages with voices, got {count}"
        );
    }
}
