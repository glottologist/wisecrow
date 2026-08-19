pub mod api;

pub use api::HttpMobileApi;
use url::Url;

use crate::application::MobileError;

/// Validated and normalized HTTPS origin for one Wisecrow server profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerOrigin {
    url: Url,
}

impl ServerOrigin {
    /// Parses an HTTPS origin and normalizes its path prefix with one trailing slash.
    ///
    /// # Errors
    ///
    /// Returns [`MobileError::InvalidInput`] for malformed or unsafe origins.
    pub fn parse(input: &str) -> Result<Self, MobileError> {
        let mut url = Url::parse(input).map_err(|_| invalid_origin())?;
        validate_origin(&url)?;
        normalize_path(&mut url)?;
        Ok(Self { url })
    }

    /// Returns the normalized origin URL.
    #[must_use]
    pub fn as_url(&self) -> &Url {
        &self.url
    }

    /// Appends validated literal path segments to this origin.
    ///
    /// # Errors
    ///
    /// Returns [`MobileError::InvalidInput`] when any segment is unsafe.
    pub fn endpoint(&self, segments: &[&str]) -> Result<Url, MobileError> {
        if segments.iter().any(|segment| !valid_segment(segment)) {
            return Err(MobileError::InvalidInput(String::from(
                "endpoint contains an invalid path segment",
            )));
        }
        let mut endpoint = self.url.clone(); // clone: each endpoint owns a URL while the normalized origin remains reusable
        endpoint
            .path_segments_mut()
            .map_err(|_| invalid_origin())?
            .pop_if_empty()
            .extend(segments);
        Ok(endpoint)
    }
}

fn validate_origin(url: &Url) -> Result<(), MobileError> {
    let invalid = url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || contains_forbidden_encoding(url.path());
    if invalid {
        return Err(invalid_origin());
    }
    Ok(())
}

fn normalize_path(url: &mut Url) -> Result<(), MobileError> {
    let trailing_slashes = url
        .path()
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'/')
        .count();
    let mut segments = url.path_segments_mut().map_err(|_| invalid_origin())?;
    for _ in 0..trailing_slashes {
        segments.pop();
    }
    segments.push("");
    Ok(())
}

fn valid_segment(segment: &str) -> bool {
    !segment.is_empty()
        && !segment.contains('/')
        && !segment.contains('\\')
        && !segment.contains("..")
        && !segment.contains('\0')
        && !contains_forbidden_encoding(segment)
}

fn contains_forbidden_encoding(value: &str) -> bool {
    let lowercase = value.to_ascii_lowercase();
    lowercase.contains("%2f") || lowercase.contains("%5c") || lowercase.contains("%00")
}

fn invalid_origin() -> MobileError {
    MobileError::InvalidInput(String::from(
        "server origin must be an HTTPS URL without credentials, query, or fragment",
    ))
}
