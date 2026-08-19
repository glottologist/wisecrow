use std::path::{Path, PathBuf};

use async_trait::async_trait;
use uuid::Uuid;

use super::{PlatformHttpRequest, PlatformHttpResponse, PlatformTransport};
use crate::{
    application::{
        BackgroundScheduler, CertificateStore, CredentialStore, FilePicker, MobileError,
    },
    storage::models::PickedFile,
};

/// Desktop fallback for the native mobile platform boundary.
pub struct DesktopPlatform {
    root: PathBuf,
}

impl DesktopPlatform {
    /// Creates a desktop platform rooted in an existing directory.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error when the root cannot be canonicalized, or an
    /// input error when it is not a directory.
    pub fn new(root: &Path) -> Result<Self, MobileError> {
        let root = root.canonicalize()?;
        if !root.is_dir() {
            return Err(MobileError::InvalidInput(String::from(
                "desktop platform root must be a directory",
            )));
        }
        Ok(Self { root })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[async_trait]
impl CredentialStore for DesktopPlatform {
    async fn load(&self, _profile_id: Uuid) -> Result<Option<String>, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn save(&self, _profile_id: Uuid, _token: &str) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn delete(&self, _profile_id: Uuid) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }
}

#[async_trait]
impl CertificateStore for DesktopPlatform {
    async fn load(&self, _profile_id: Uuid) -> Result<Option<Vec<u8>>, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn save(&self, _profile_id: Uuid, _certificate: &[u8]) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn delete(&self, _profile_id: Uuid) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }
}

#[async_trait]
impl FilePicker for DesktopPlatform {
    async fn pick_pdf(&self, _maximum_bytes: u64) -> Result<Option<PickedFile>, MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn pick_certificate(
        &self,
        _maximum_bytes: u64,
    ) -> Result<Option<PickedFile>, MobileError> {
        Err(MobileError::Unsupported)
    }
}

#[async_trait]
impl BackgroundScheduler for DesktopPlatform {
    async fn schedule_sync(&self, _profile_id: Uuid) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }

    async fn cancel_sync(&self, _profile_id: Uuid) -> Result<(), MobileError> {
        Err(MobileError::Unsupported)
    }
}

#[async_trait]
impl PlatformTransport for DesktopPlatform {
    async fn execute(
        &self,
        _request: &PlatformHttpRequest<'_>,
    ) -> Result<PlatformHttpResponse, MobileError> {
        Err(MobileError::Unsupported)
    }
}
