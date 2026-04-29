use crate::errors::WisecrowError;

/// Parses an Advanced SubStation Alpha (`.ass`) or SubStation Alpha (`.ssa`)
/// subtitle file body into cue text lines. The format uses comma-separated
/// `Dialogue:` rows with text in the 10th field; we strip the 9 leading
/// fields and override codes (`{...}`) and convert `\N` line breaks.
///
/// # Errors
///
/// Currently never errors; returns `Result` for future-proofing.
pub fn parse_ass(content: &str) -> Result<Vec<String>, WisecrowError> {
    let stripped = content.strip_prefix('\u{feff}').unwrap_or(content);
    let normalised = stripped.replace("\r\n", "\n");
    let mut out = Vec::new();
    for line in normalised.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("Dialogue:") else {
            continue;
        };
        // Dialogue line has 9 comma-separated fields before the text:
        // Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
        let parts: Vec<&str> = rest.splitn(10, ',').collect();
        if parts.len() < 10 {
            continue;
        }
        let text_field = parts[9];
        let cleaned = strip_ass_overrides(text_field).replace("\\N", "\n");
        let cleaned = cleaned.trim();
        if !cleaned.is_empty() {
            out.push(cleaned.to_owned());
        }
    }
    Ok(out)
}

fn strip_ass_overrides(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_override = false;
    for ch in text.chars() {
        match ch {
            '{' => in_override = true,
            '}' => in_override = false,
            _ if !in_override => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Parses a WebVTT subtitle file body into cue text lines. Handles BOM, CRLF,
/// optional cue identifiers, and timestamps using `.` separators.
///
/// # Errors
///
/// Currently never errors; returns `Result` for future-proofing.
pub fn parse_vtt(content: &str) -> Result<Vec<String>, WisecrowError> {
    let stripped = content.strip_prefix('\u{feff}').unwrap_or(content);
    let normalised = stripped.replace("\r\n", "\n");
    let body = normalised
        .trim_start()
        .trim_start_matches("WEBVTT")
        .trim_start();
    let mut out = Vec::new();
    for block in body.split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        let Some(idx) = lines.iter().position(|l| l.contains("-->")) else {
            continue;
        };
        let cue_lines: Vec<&str> = lines[idx.saturating_add(1)..]
            .iter()
            .copied()
            .filter(|l| !l.is_empty())
            .collect();
        if !cue_lines.is_empty() {
            out.push(cue_lines.join("\n"));
        }
    }
    Ok(out)
}

/// Parses an SRT subtitle file body into the cue text lines, dropping cue
/// indices and timestamps. Handles UTF-8 BOM and `\r\n` endings.
///
/// # Errors
///
/// Currently never errors; returns `Result` for future-proofing.
pub fn parse_srt(content: &str) -> Result<Vec<String>, WisecrowError> {
    let stripped = content.strip_prefix('\u{feff}').unwrap_or(content);
    let normalised = stripped.replace("\r\n", "\n");
    let mut out = Vec::new();
    for block in normalised.split("\n\n") {
        let lines: Vec<&str> = block.lines().collect();
        if lines.len() < 3 {
            continue;
        }
        let cue: Vec<&str> = lines[2..]
            .iter()
            .copied()
            .filter(|l| !l.is_empty())
            .collect();
        if !cue.is_empty() {
            out.push(cue.join("\n"));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parse_srt_basic() {
        let s = "1\n00:00:01,000 --> 00:00:02,000\nHola amigo.\n\n2\n00:00:03,000 --> 00:00:04,000\n¿Cómo estás?\n";
        let cues = parse_srt(s).expect("parse failed");
        assert_eq!(cues, vec!["Hola amigo.", "¿Cómo estás?"]);
    }

    #[test]
    fn parse_srt_handles_bom_and_crlf() {
        let s = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,000\r\nHola.\r\n\r\n";
        let cues = parse_srt(s).expect("parse failed");
        assert_eq!(cues, vec!["Hola."]);
    }

    #[test]
    fn parse_srt_multiline_cue() {
        let s = "1\n00:00:01,000 --> 00:00:02,000\nLine one\nLine two\n";
        let cues = parse_srt(s).expect("parse failed");
        assert_eq!(cues, vec!["Line one\nLine two"]);
    }

    #[test]
    fn parse_vtt_basic() {
        let s = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nHola amigo.\n\n00:00:03.000 --> 00:00:04.000\n¿Cómo estás?\n";
        let cues = parse_vtt(s).expect("parse failed");
        assert_eq!(cues, vec!["Hola amigo.", "¿Cómo estás?"]);
    }

    #[test]
    fn parse_vtt_with_optional_id() {
        let s = "WEBVTT\n\ncue1\n00:00:01.000 --> 00:00:02.000\nHola.\n";
        let cues = parse_vtt(s).expect("parse failed");
        assert_eq!(cues, vec!["Hola."]);
    }

    #[test]
    fn parse_ass_basic() {
        let s = "[Script Info]\nTitle: test\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Hola amigo.\nDialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,¿Cómo estás?\n";
        let cues = parse_ass(s).expect("parse failed");
        assert_eq!(cues, vec!["Hola amigo.", "¿Cómo estás?"]);
    }

    #[test]
    fn parse_ass_strips_override_tags_and_handles_line_break() {
        let s = "[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\b1}Hola{\\b0}\\NLine two\n";
        let cues = parse_ass(s).expect("parse failed");
        assert_eq!(cues, vec!["Hola\nLine two"]);
    }

    #[test]
    fn parse_ass_ignores_comment_and_format_lines() {
        let s = "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nComment: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,not a dialogue\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,real cue\n";
        let cues = parse_ass(s).expect("parse failed");
        assert_eq!(cues, vec!["real cue"]);
    }

    proptest! {
        #[test]
        fn srt_never_panics(s in ".*") {
            let _ = parse_srt(&s);
        }

        #[test]
        fn vtt_never_panics(s in ".*") {
            let _ = parse_vtt(&s);
        }

        #[test]
        fn ass_never_panics(s in ".*") {
            let _ = parse_ass(&s);
        }
    }
}
