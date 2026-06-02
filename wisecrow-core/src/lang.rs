//! Shared language-code helpers.

/// Maximum length of a well-formed language code.
pub const MAX_LANG_CODE_LEN: usize = 10;

/// Returns `true` if `code` is a well-formed language code: non-empty, at most
/// [`MAX_LANG_CODE_LEN`] characters, and ASCII alphanumeric. This is a syntactic
/// check only — use `cli::is_supported_language` to check membership of the
/// supported set.
#[must_use]
pub fn is_valid_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= MAX_LANG_CODE_LEN
        && code.chars().all(|c| c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_well_formed_codes() {
        assert!(is_valid_code("en"));
        assert!(is_valid_code("zh"));
        assert!(is_valid_code("ceb"));
    }

    #[test]
    fn rejects_malformed_codes() {
        assert!(!is_valid_code("")); // empty
        assert!(!is_valid_code("toolonglangcode")); // > 10 chars
        assert!(!is_valid_code("en-US")); // non-alphanumeric
        assert!(!is_valid_code("e n")); // whitespace
    }
}
