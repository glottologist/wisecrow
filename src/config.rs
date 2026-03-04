use std::borrow::Cow;

use serde::Deserialize;
use url::Url;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::errors::WisecrowError;

#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecureString(String);

impl<'de> Deserialize<'de> for SecureString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(SecureString)
    }
}

impl SecureString {
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl From<String> for SecureString {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[derive(Deserialize, Clone)]
pub struct Config {
    pub db_url: Option<SecureString>,
    pub db_address: Option<String>,
    pub db_name: Option<String>,
    pub db_user: Option<String>,
    pub db_password: Option<SecureString>,
    pub unsplash_api_key: Option<SecureString>,
}

impl Config {
    /// Returns the database URL, either directly from `db_url` or assembled
    /// from the component fields.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::ConfigurationError`] if neither `db_url` nor
    /// a complete set of component fields is present, or if the URL cannot be
    /// constructed.
    pub fn database_url(&self) -> Result<Cow<'_, str>, WisecrowError> {
        if let Some(url) = &self.db_url {
            return Ok(Cow::Borrowed(url.expose()));
        }

        match (
            &self.db_address,
            &self.db_name,
            &self.db_user,
            &self.db_password,
        ) {
            (Some(addr), Some(name), Some(user), Some(pass)) => {
                let mut url = Url::parse("postgres://localhost")
                    .map_err(|e| WisecrowError::ConfigurationError(e.to_string()))?;
                url.set_host(Some(addr))
                    .map_err(|e| WisecrowError::ConfigurationError(e.to_string()))?;
                url.set_username(user).map_err(|()| {
                    WisecrowError::ConfigurationError("Failed to set database username".to_string())
                })?;
                url.set_password(Some(pass.expose())).map_err(|()| {
                    WisecrowError::ConfigurationError("Failed to set database password".to_string())
                })?;
                url.set_path(&format!("/{name}"));
                Ok(Cow::Owned(url.into()))
            }
            _ => Err(WisecrowError::ConfigurationError(
                "Either db_url or all of db_address, db_name, db_user, db_password must be set"
                    .to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_url(url: &str) -> Config {
        Config {
            db_url: Some(SecureString::from(url.to_owned())),
            db_address: None,
            db_name: None,
            db_user: None,
            db_password: None,
            unsplash_api_key: None,
        }
    }

    fn config_with_components(addr: &str, name: &str, user: &str, pass: &str) -> Config {
        Config {
            db_url: None,
            db_address: Some(addr.to_owned()),
            db_name: Some(name.to_owned()),
            db_user: Some(user.to_owned()),
            db_password: Some(SecureString::from(pass.to_owned())),
            unsplash_api_key: None,
        }
    }

    #[test]
    fn database_url_from_direct_url() {
        let config = config_with_url("postgres://user:pass@host/db");
        let url = config.database_url().unwrap();
        assert_eq!(url.as_ref(), "postgres://user:pass@host/db");
    }

    #[test]
    fn database_url_from_components() {
        let config = config_with_components("localhost", "wisecrow", "admin", "secret");
        let url = config.database_url().unwrap();
        assert!(url.contains("admin"));
        assert!(url.contains("secret"));
        assert!(url.contains("localhost"));
        assert!(url.contains("/wisecrow"));
    }

    #[test]
    fn database_url_missing_components_errors() {
        let config = Config {
            db_url: None,
            db_address: Some("localhost".to_owned()),
            db_name: None,
            db_user: None,
            db_password: None,
            unsplash_api_key: None,
        };
        assert!(config.database_url().is_err());
    }

    #[test]
    fn direct_url_takes_priority_over_components() {
        let config = Config {
            db_url: Some(SecureString::from("postgres://direct@host/db".to_owned())),
            db_address: Some("other-host".to_owned()),
            db_name: Some("other-db".to_owned()),
            db_user: Some("other-user".to_owned()),
            db_password: Some(SecureString::from("other-pass".to_owned())),
            unsplash_api_key: None,
        };
        let url = config.database_url().unwrap();
        assert_eq!(url.as_ref(), "postgres://direct@host/db");
    }
}
