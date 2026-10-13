//! The language pairs a corpus or grammar wake is allowed to touch.

use std::path::Path;

use crate::cli::is_supported_language;
use crate::errors::WisecrowError;

/// One configured pair. Absent `corpora` means the shared selection policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairSpec {
    pub native: String,
    pub foreign: String,
    pub corpora: Option<Vec<String>>,
}

/// Reads `path` as a list of `[[pair]]` tables.
///
/// # Errors
///
/// Returns an error when the file cannot be read, the tables are malformed,
/// or a language code is not supported.
pub fn load_pairs(path: &Path) -> Result<Vec<PairSpec>, WisecrowError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        WisecrowError::InvalidInput(format!("{} cannot be read: {error}", path.display()))
    })?;
    parse_pairs(&text)
}

/// Parses the pair file.
///
/// # Errors
///
/// Returns an error when a table is malformed or a language is not supported.
pub fn parse_pairs(text: &str) -> Result<Vec<PairSpec>, WisecrowError> {
    let mut pairs = Vec::new();
    let mut current: Option<Draft> = None;

    for (number, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let line_number = number.saturating_add(1);
        if line == "[[pair]]" {
            if let Some(draft) = current.take() {
                pairs.push(finish(draft)?);
            }
            current = Some(Draft::default());
            continue;
        }
        let Some(draft) = current.as_mut() else {
            return Err(invalid(line_number, "expected [[pair]]"));
        };
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| invalid(line_number, "expected key = value"))?;
        match key.trim() {
            "native" => draft.native = Some(parse_string(value, line_number)?),
            "foreign" => draft.foreign = Some(parse_string(value, line_number)?),
            "corpora" => draft.corpora = Some(parse_list(value, line_number)?),
            other => {
                return Err(invalid(
                    line_number,
                    &format!("unknown key {other}; expected native, foreign, or corpora"),
                ))
            }
        }
    }

    if let Some(draft) = current {
        pairs.push(finish(draft)?);
    }
    if pairs.is_empty() {
        return Err(WisecrowError::InvalidInput(
            "language pair file names no pairs".to_owned(),
        ));
    }
    Ok(pairs)
}

#[derive(Default)]
struct Draft {
    native: Option<String>,
    foreign: Option<String>,
    corpora: Option<Vec<String>>,
}

fn finish(draft: Draft) -> Result<PairSpec, WisecrowError> {
    let native = draft
        .native
        .ok_or_else(|| WisecrowError::InvalidInput("a pair is missing native".to_owned()))?;
    let foreign = draft
        .foreign
        .ok_or_else(|| WisecrowError::InvalidInput("a pair is missing foreign".to_owned()))?;
    if !is_supported_language(&native) {
        return Err(WisecrowError::InvalidInput(format!(
            "Unsupported native language: {native}"
        )));
    }
    if !is_supported_language(&foreign) {
        return Err(WisecrowError::InvalidInput(format!(
            "Unsupported foreign language: {foreign}"
        )));
    }
    if native == foreign {
        return Err(WisecrowError::InvalidInput(
            "native and foreign language must differ".to_owned(),
        ));
    }
    Ok(PairSpec {
        native,
        foreign,
        corpora: draft.corpora,
    })
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    for (index, byte) in line.bytes().enumerate() {
        match byte {
            b'"' => in_string = !in_string,
            b'#' if !in_string => return &line[..index],
            _ => {}
        }
    }
    line
}

fn parse_string(value: &str, line_number: usize) -> Result<String, WisecrowError> {
    let value = value.trim();
    let inner = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .ok_or_else(|| invalid(line_number, "expected a quoted string"))?;
    if inner.contains(['\\', '"']) {
        return Err(invalid(line_number, "escapes are not accepted"));
    }
    if inner.is_empty() {
        return Err(invalid(line_number, "expected a non-empty string"));
    }
    Ok(inner.to_owned())
}

fn parse_list(value: &str, line_number: usize) -> Result<Vec<String>, WisecrowError> {
    let value = value.trim();
    let inner = value
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .ok_or_else(|| invalid(line_number, "expected a list"))?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for item in inner.split(',') {
        names.push(parse_string(item, line_number)?);
    }
    Ok(names)
}

fn invalid(line_number: usize, reason: &str) -> WisecrowError {
    WisecrowError::InvalidInput(format!("language pair file line {line_number}: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::{load_pairs, parse_pairs};
    use std::path::Path;

    #[test]
    fn a_missing_corpora_key_is_the_policy_and_an_empty_list_is_not() {
        let pairs = parse_pairs(
            r#"
            [[pair]]
            native = "en"
            foreign = "gd"

            [[pair]]
            native = "en"
            foreign = "fr"
            corpora = []
            "#,
        )
        .expect("pairs");
        assert!(pairs[0].corpora.is_none());
        assert_eq!(pairs[1].corpora.as_deref(), Some([].as_slice()));
    }

    #[test]
    fn an_explicit_list_replaces_the_policy() {
        let pairs = parse_pairs(
            r#"
            [[pair]]
            native = "en"
            foreign = "es"
            corpora = ["nllb", "paracrawl"]
            "#,
        )
        .expect("pairs");
        assert_eq!(
            pairs[0].corpora.as_deref(),
            Some(["nllb".to_owned(), "paracrawl".to_owned()].as_slice())
        );
    }

    #[test]
    fn a_malformed_pair_is_rejected() {
        assert!(parse_pairs("native = \"en\"\n").is_err());
        assert!(parse_pairs("[[pair]]\nnative = \"en\"\n").is_err());
        assert!(parse_pairs("[[pair]]\nnative = \"zz\"\nforeign = \"es\"\n").is_err());
    }

    #[test]
    fn the_deployed_file_names_the_four_initial_pairs() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../deploy/language-pairs.toml");
        let pairs = load_pairs(&path).expect("deployed pairs");
        let codes: Vec<&str> = pairs.iter().map(|pair| pair.foreign.as_str()).collect();
        assert_eq!(codes, ["gd", "fr", "it", "es"]);
        assert!(pairs
            .iter()
            .all(|pair| pair.native == "en" && pair.corpora.is_none()));
    }
}
