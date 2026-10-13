//! Which OPUS collections a configured pair is allowed to ingest.

use crate::errors::WisecrowError;
use crate::lang;

const BASE_CORPORA: [&str; 4] = ["open_subtitles", "cc_aligned", "paracrawl", "nllb"];

/// Collections to ingest for `foreign`.
///
/// An explicit `selection` replaces the policy. French, Italian, Spanish and
/// every other language take the four collections that do not drown a
/// frequency list. Scottish Gaelic also takes CCMatrix, whose release is small
/// enough to keep.
///
/// # Errors
///
/// Returns an error when `foreign` is not a well-formed language code.
pub fn corpora_for<'a>(
    foreign: &str,
    selection: Option<&'a [&'a str]>,
) -> Result<Vec<&'a str>, WisecrowError> {
    if !lang::is_valid_code(foreign) {
        return Err(WisecrowError::InvalidInput(format!(
            "Invalid language code: {foreign}"
        )));
    }
    if let Some(selection) = selection {
        return Ok(selection.to_vec());
    }
    let mut corpora = BASE_CORPORA.to_vec();
    if foreign == "gd" {
        corpora.push("cc_matrix");
    }
    Ok(corpora)
}

#[cfg(test)]
mod tests {
    use super::corpora_for;

    #[test]
    fn major_languages_skip_ccmatrix_and_gaelic_keeps_it() {
        let spanish = corpora_for("es", None).expect("es");
        assert_eq!(
            spanish,
            vec!["open_subtitles", "cc_aligned", "paracrawl", "nllb"]
        );
        assert!(!spanish.contains(&"cc_matrix"));
        assert!(corpora_for("gd", None).expect("gd").contains(&"cc_matrix"));
        assert_eq!(
            corpora_for("fr", Some(&["nllb"])).expect("fr"),
            vec!["nllb"]
        );
    }

    #[test]
    fn a_malformed_code_is_rejected() {
        assert!(corpora_for("not a code", None).is_err());
    }
}
