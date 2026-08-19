mod desktop;

#[cfg(target_os = "android")]
mod android;

use async_trait::async_trait;
use url::Url;
use uuid::Uuid;

use crate::application::MobileError;

pub use desktop::DesktopPlatform;

#[cfg(target_os = "android")]
pub use android::AndroidPlatform;

/// HTTP method supported by the native transport boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformHttpMethod {
    Get,
    Post,
}

/// Borrowed HTTP request passed to the platform's secure client.
#[derive(Debug)]
pub struct PlatformHttpRequest<'a> {
    pub profile_id: Uuid,
    pub origin: &'a Url,
    pub url: &'a Url,
    pub method: PlatformHttpMethod,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: &'a [u8],
    pub maximum_response_bytes: u64,
}

/// Owned HTTP response returned by the platform's secure client.
#[derive(Debug, PartialEq, Eq)]
pub struct PlatformHttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Executes requests through the platform-specific secure HTTP client.
#[async_trait]
pub trait PlatformTransport: Send + Sync {
    async fn execute(
        &self,
        request: &PlatformHttpRequest<'_>,
    ) -> Result<PlatformHttpResponse, MobileError>;
}
