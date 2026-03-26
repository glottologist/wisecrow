use std::path::Path;

use crate::errors::WisecrowError;

/// Maps wisecrow language codes to MS Edge TTS voice names.
#[must_use]
pub fn voice_for_language(lang_code: &str) -> Option<&'static str> {
    match lang_code {
        "af" => Some("af-ZA-AdriNeural"),
        "am" => Some("am-ET-AmehaNeural"),
        "ar" => Some("ar-SA-HamedNeural"),
        "bg" => Some("bg-BG-BorislavNeural"),
        "bn" => Some("bn-IN-BashkarNeural"),
        "bs" => Some("bs-BA-GoranNeural"),
        "ca" => Some("ca-ES-EnricNeural"),
        "cs" => Some("cs-CZ-AntoninNeural"),
        "cy" => Some("cy-GB-AledNeural"),
        "da" => Some("da-DK-JeppeNeural"),
        "de" => Some("de-DE-ConradNeural"),
        "el" => Some("el-GR-NestorasNeural"),
        "en" => Some("en-US-GuyNeural"),
        "es" => Some("es-ES-AlvaroNeural"),
        "et" => Some("et-EE-KertNeural"),
        "fa" => Some("fa-IR-FaridNeural"),
        "fi" => Some("fi-FI-HarriNeural"),
        "fr" => Some("fr-FR-HenriNeural"),
        "ga" => Some("ga-IE-ColmNeural"),
        "gl" => Some("gl-ES-RoiNeural"),
        "gu" => Some("gu-IN-NiranjanNeural"),
        "he" => Some("he-IL-AvriNeural"),
        "hi" => Some("hi-IN-MadhurNeural"),
        "hr" => Some("hr-HR-SreckoNeural"),
        "hu" => Some("hu-HU-TamasNeural"),
        "id" => Some("id-ID-ArdiNeural"),
        "is" => Some("is-IS-GunnarNeural"),
        "it" => Some("it-IT-DiegoNeural"),
        "ja" => Some("ja-JP-KeitaNeural"),
        "jv" => Some("jv-ID-DimasNeural"),
        "ka" => Some("ka-GE-GiorgiNeural"),
        "kk" => Some("kk-KZ-DauletNeural"),
        "km" => Some("km-KH-PisethNeural"),
        "kn" => Some("kn-IN-GaganNeural"),
        "ko" => Some("ko-KR-InJoonNeural"),
        "lo" => Some("lo-LA-ChanthavongNeural"),
        "lt" => Some("lt-LT-LeonasNeural"),
        "lv" => Some("lv-LV-NilsNeural"),
        "mk" => Some("mk-MK-AleksandarNeural"),
        "ml" => Some("ml-IN-MidhunNeural"),
        "mn" => Some("mn-MN-BataaNeural"),
        "mr" => Some("mr-IN-ManoharNeural"),
        "ms" => Some("ms-MY-OsmanNeural"),
        "my" => Some("my-MM-ThihaNeural"),
        "ne" => Some("ne-NP-SagarNeural"),
        "nl" => Some("nl-NL-MaartenNeural"),
        "no" => Some("nb-NO-FinnNeural"),
        "pa" => Some("pa-IN-GurdeepNeural"),
        "pl" => Some("pl-PL-MarekNeural"),
        "ps" => Some("ps-AF-GulNawazNeural"),
        "pt" => Some("pt-BR-AntonioNeural"),
        "ro" => Some("ro-RO-EmilNeural"),
        "ru" => Some("ru-RU-DmitryNeural"),
        "si" => Some("si-LK-SameeraNeural"),
        "sk" => Some("sk-SK-LukasNeural"),
        "sl" => Some("sl-SI-RokNeural"),
        "so" => Some("so-SO-MuuseNeural"),
        "sq" => Some("sq-AL-IlirNeural"),
        "sr" => Some("sr-RS-NicholasNeural"),
        "su" => Some("su-ID-JajangNeural"),
        "sv" => Some("sv-SE-MattiasNeural"),
        "sw" => Some("sw-KE-RafikiNeural"),
        "ta" => Some("ta-IN-ValluvarNeural"),
        "te" => Some("te-IN-MohanNeural"),
        "th" => Some("th-TH-NiwatNeural"),
        "tl" => Some("fil-PH-BlessicaNeural"),
        "tr" => Some("tr-TR-AhmetNeural"),
        "uk" => Some("uk-UA-OstapNeural"),
        "ur" => Some("ur-PK-AsadNeural"),
        "uz" => Some("uz-UZ-SardorNeural"),
        "vi" => Some("vi-VN-NamMinhNeural"),
        "zh" => Some("zh-CN-YunxiNeural"),
        "zu" => Some("zu-ZA-ThembaNeural"),
        _ => None,
    }
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

    let text = text.to_owned();
    let voice = voice.to_owned();

    tokio::task::spawn_blocking(move || {
        let mut tts = msedge_tts::tts::client::connect()
            .map_err(|e| WisecrowError::MediaError(format!("TTS connection failed: {e}")))?;

        let config = msedge_tts::tts::SpeechConfig::from(
            &msedge_tts::voice::get_voices_list()
                .map_err(|e| WisecrowError::MediaError(format!("Failed to get voices: {e}")))?
                .into_iter()
                .find(|v| v.short_name == voice)
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
