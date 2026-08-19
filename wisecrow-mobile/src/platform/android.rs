use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

use super::{PlatformHttpMethod, PlatformHttpRequest, PlatformHttpResponse, PlatformTransport};
use crate::{
    application::{
        BackgroundScheduler, CertificateStore, CredentialStore, FilePicker, MobileError,
    },
    storage::models::PickedFile,
};

#[manganis::ffi("android/platform")]
extern "Kotlin" {
    pub type WisecrowPlatform;

    fn app_data_path(this: &WisecrowPlatform) -> String;
    fn credential_load(this: &WisecrowPlatform, profile_id: &str) -> String;
    fn credential_save(this: &WisecrowPlatform, profile_id: &str, token: &str) -> String;
    fn credential_delete(this: &WisecrowPlatform, profile_id: &str) -> String;
    fn ca_import(this: &WisecrowPlatform, profile_id: &str, certificate_base64: &str) -> String;
    fn ca_delete(this: &WisecrowPlatform, profile_id: &str) -> String;
    fn ca_load(this: &WisecrowPlatform, profile_id: &str) -> String;
    fn picker_start(this: &WisecrowPlatform, kind: &str, maximum_bytes: u64) -> String;
    fn picker_poll(this: &WisecrowPlatform, request_id: &str) -> String;
    fn picker_cancel(this: &WisecrowPlatform, request_id: &str) -> String;
    fn http_start(this: &WisecrowPlatform, request_json: &str) -> String;
    fn http_poll(this: &WisecrowPlatform, request_id: &str) -> String;
    fn http_cancel(this: &WisecrowPlatform, request_id: &str) -> String;
    fn sync_schedule(this: &WisecrowPlatform, profile_id: &str) -> String;
    fn sync_cancel(this: &WisecrowPlatform, profile_id: &str) -> String;
    fn connectivity_state(this: &WisecrowPlatform) -> String;
}

/// Android implementation of the mobile platform boundary.
pub struct AndroidPlatform {
    native: WisecrowPlatform,
}

impl AndroidPlatform {
    /// Connects to the Kotlin platform object owned by the active Android activity.
    ///
    /// # Errors
    ///
    /// Returns a permanent platform error when JNI or the activity is unavailable.
    pub fn new() -> Result<Self, MobileError> {
        let native = WisecrowPlatform::new().map_err(|_| MobileError::Permanent)?;
        Ok(Self { native })
    }

    /// Returns the app-private data path reported by Android.
    ///
    /// # Errors
    ///
    /// Returns a typed native-envelope or JNI error.
    pub fn app_data_path(&self) -> Result<String, MobileError> {
        native_value(app_data_path(&self.native))
    }
}

#[derive(Deserialize)]
struct NativeEnvelope {
    status: String,
    code: Option<String>,
    value: Option<String>,
    request_id: Option<String>,
    display_name: Option<String>,
    media_type: Option<String>,
    bytes_base64: Option<String>,
    http_status: Option<u16>,
    headers: Option<Vec<(String, String)>>,
    body_base64: Option<String>,
}

#[derive(Serialize)]
struct NativeHttpRequest<'a> {
    profile_id: Uuid,
    origin: &'a str,
    url: &'a str,
    method: &'static str,
    headers: &'a [(&'a str, &'a str)],
    body_base64: String,
    maximum_response_bytes: u64,
}

fn native_envelope(result: Result<String, String>) -> Result<NativeEnvelope, MobileError> {
    let body = result.map_err(|_| MobileError::Permanent)?;
    let envelope: NativeEnvelope = serde_json::from_str(&body)?;
    if envelope.status == "ERROR" {
        return Err(native_error(envelope.code.as_deref()));
    }
    Ok(envelope)
}

fn native_error(code: Option<&str>) -> MobileError {
    match code {
        Some("NOT_IMPLEMENTED") => MobileError::Unsupported,
        Some("INVALID_INPUT" | "FILE_TOO_LARGE" | "WRONG_MIME_TYPE" | "INVALID_FILE") => {
            MobileError::InvalidInput(String::from("the native operation input is invalid"))
        }
        Some("INVALID_CERTIFICATE" | "NOT_CERTIFICATE_AUTHORITY") => {
            MobileError::InvalidInput(String::from("the imported certificate is invalid"))
        }
        Some(
            "KEYSTORE_UNAVAILABLE"
            | "CREDENTIAL_CORRUPT"
            | "AUTHENTICATION_FAILED"
            | "STORAGE_READ_FAILED"
            | "STORAGE_WRITE_FAILED"
            | "DELETE_FAILED",
        ) => MobileError::Credentials,
        Some("BUSY" | "NETWORK_FAILED") => MobileError::Retryable,
        Some(
            "TLS_FAILED" | "CERTIFICATE_UNAVAILABLE" | "RESPONSE_TOO_LARGE" | "RESPONSE_INVALID",
        ) => MobileError::Permanent,
        _ => MobileError::Permanent,
    }
}

fn native_unit(result: Result<String, String>) -> Result<(), MobileError> {
    native_envelope(result).map(|_| ())
}

fn native_value(result: Result<String, String>) -> Result<String, MobileError> {
    native_envelope(result)?.value.ok_or(MobileError::Permanent)
}

fn native_optional_value(result: Result<String, String>) -> Result<Option<String>, MobileError> {
    Ok(native_envelope(result)?.value)
}

fn profile_id_string(profile_id: Uuid) -> String {
    profile_id.hyphenated().to_string()
}

type CancelNativeRequest = fn(&WisecrowPlatform, &str) -> Result<String, String>;

struct NativeRequestGuard<'a> {
    native: &'a WisecrowPlatform,
    request_id: String,
    cancel: CancelNativeRequest,
    terminal: bool,
}

impl<'a> NativeRequestGuard<'a> {
    fn new(native: &'a WisecrowPlatform, request_id: String, cancel: CancelNativeRequest) -> Self {
        Self {
            native,
            request_id,
            cancel,
            terminal: false,
        }
    }

    fn complete(&mut self) {
        self.terminal = true;
    }
}

impl Drop for NativeRequestGuard<'_> {
    fn drop(&mut self) {
        if self.terminal {
            return;
        }
        if native_unit((self.cancel)(self.native, &self.request_id)).is_err() {
            tracing::warn!("failed to cancel an unfinished Android native request");
        }
    }
}

#[async_trait]
impl CredentialStore for AndroidPlatform {
    async fn load(&self, profile_id: Uuid) -> Result<Option<String>, MobileError> {
        let profile_id = profile_id_string(profile_id);
        native_optional_value(credential_load(&self.native, &profile_id))
    }

    async fn save(&self, profile_id: Uuid, token: &str) -> Result<(), MobileError> {
        let profile_id = profile_id_string(profile_id);
        native_unit(credential_save(&self.native, &profile_id, token))
    }

    async fn delete(&self, profile_id: Uuid) -> Result<(), MobileError> {
        let profile_id = profile_id_string(profile_id);
        native_unit(credential_delete(&self.native, &profile_id))
    }
}

#[async_trait]
impl CertificateStore for AndroidPlatform {
    async fn load(&self, profile_id: Uuid) -> Result<Option<Vec<u8>>, MobileError> {
        let profile_id = profile_id_string(profile_id);
        let encoded = match native_optional_value(ca_load(&self.native, &profile_id))? {
            Some(encoded) => encoded,
            None => return Ok(None),
        };
        validate_encoded_size(&encoded, MAX_CERTIFICATE_BYTES)?;
        let certificate = STANDARD
            .decode(encoded)
            .map_err(|_| MobileError::Permanent)?;
        if u64::try_from(certificate.len()).map_err(|_| MobileError::Permanent)?
            > MAX_CERTIFICATE_BYTES
        {
            return Err(MobileError::Permanent);
        }
        Ok(Some(certificate))
    }

    async fn save(&self, profile_id: Uuid, certificate: &[u8]) -> Result<(), MobileError> {
        validate_maximum(
            u64::try_from(certificate.len()).map_err(|_| MobileError::Permanent)?,
            MAX_CERTIFICATE_BYTES,
        )?;
        let profile_id = profile_id_string(profile_id);
        let certificate = STANDARD.encode(certificate);
        let fingerprint = native_value(ca_import(&self.native, &profile_id, &certificate))?;
        validate_fingerprint(&fingerprint)
    }

    async fn delete(&self, profile_id: Uuid) -> Result<(), MobileError> {
        let profile_id = profile_id_string(profile_id);
        native_unit(ca_delete(&self.native, &profile_id))
    }
}

fn validate_fingerprint(fingerprint: &str) -> Result<(), MobileError> {
    if fingerprint.len() != SHA256_HEX_BYTES
        || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(MobileError::Permanent);
    }
    Ok(())
}

#[async_trait]
impl FilePicker for AndroidPlatform {
    async fn pick_pdf(&self, maximum_bytes: u64) -> Result<Option<PickedFile>, MobileError> {
        self.pick("pdf", validate_maximum(maximum_bytes, MAX_PDF_BYTES)?)
            .await
    }

    async fn pick_certificate(
        &self,
        maximum_bytes: u64,
    ) -> Result<Option<PickedFile>, MobileError> {
        self.pick(
            "certificate",
            validate_maximum(maximum_bytes, MAX_CERTIFICATE_BYTES)?,
        )
        .await
    }
}

impl AndroidPlatform {
    async fn pick(
        &self,
        kind: &str,
        maximum_bytes: u64,
    ) -> Result<Option<PickedFile>, MobileError> {
        let envelope = native_envelope(picker_start(&self.native, kind, maximum_bytes))?;
        if envelope.status != "PENDING" {
            return Err(MobileError::Permanent);
        }
        let request_id = envelope.request_id.ok_or(MobileError::Permanent)?;
        validate_request_id(&request_id)?;
        let mut request = NativeRequestGuard::new(&self.native, request_id, picker_cancel);
        self.poll_picker(&mut request, kind, maximum_bytes).await
    }

    async fn poll_picker(
        &self,
        request: &mut NativeRequestGuard<'_>,
        kind: &str,
        maximum_bytes: u64,
    ) -> Result<Option<PickedFile>, MobileError> {
        loop {
            let envelope = native_envelope(picker_poll(&self.native, &request.request_id))?;
            match envelope.status.as_str() {
                "PENDING" => tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await,
                "CANCELLED" => {
                    request.complete();
                    return Ok(None);
                }
                "READY" => {
                    request.complete();
                    let file = picked_file(envelope, kind, maximum_bytes)?;
                    return Ok(Some(file));
                }
                _ => return Err(MobileError::Permanent),
            }
        }
    }
}

fn validate_maximum(requested: u64, platform_limit: u64) -> Result<u64, MobileError> {
    if requested == 0 || requested > platform_limit {
        return Err(MobileError::InvalidInput(String::from(
            "the selected file size limit is invalid",
        )));
    }
    Ok(requested)
}

fn validate_request_id(request_id: &str) -> Result<(), MobileError> {
    if request_id.len() != UUID_TEXT_BYTES || Uuid::parse_str(request_id).is_err() {
        return Err(MobileError::Permanent);
    }
    Ok(())
}

fn picked_file(
    envelope: NativeEnvelope,
    kind: &str,
    maximum_bytes: u64,
) -> Result<PickedFile, MobileError> {
    let display_name = envelope.display_name.ok_or(MobileError::Permanent)?;
    if display_name.is_empty() || display_name.chars().count() > MAX_NAME_CHARS {
        return Err(MobileError::Permanent);
    }
    let media_type = envelope.media_type.ok_or(MobileError::Permanent)?;
    validate_media_type(kind, &media_type)?;
    let encoded = envelope.bytes_base64.ok_or(MobileError::Permanent)?;
    validate_encoded_size(&encoded, maximum_bytes)?;
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| MobileError::Permanent)?;
    if u64::try_from(bytes.len()).map_err(|_| MobileError::Permanent)? > maximum_bytes {
        return Err(MobileError::Permanent);
    }
    Ok(PickedFile {
        display_name,
        media_type,
        bytes,
    })
}

fn validate_media_type(kind: &str, media_type: &str) -> Result<(), MobileError> {
    let valid = match kind {
        "pdf" => media_type == "application/pdf",
        "certificate" => matches!(
            media_type,
            "application/pkix-cert" | "application/x-x509-ca-cert" | "application/x-pem-file"
        ),
        _ => false,
    };
    if !valid {
        return Err(MobileError::Permanent);
    }
    Ok(())
}

fn validate_encoded_size(encoded: &str, maximum_bytes: u64) -> Result<(), MobileError> {
    let encoded_limit = maximum_bytes
        .checked_add(2)
        .and_then(|bytes| bytes.checked_div(3))
        .and_then(|groups| groups.checked_mul(4))
        .ok_or(MobileError::Permanent)?;
    let encoded_limit = usize::try_from(encoded_limit).map_err(|_| MobileError::Permanent)?;
    if encoded.len() > encoded_limit {
        return Err(MobileError::Permanent);
    }
    Ok(())
}

#[async_trait]
impl BackgroundScheduler for AndroidPlatform {
    async fn schedule_sync(&self, profile_id: Uuid) -> Result<(), MobileError> {
        let profile_id = profile_id_string(profile_id);
        native_unit(connectivity_state(&self.native))?;
        native_unit(sync_schedule(&self.native, &profile_id))
    }

    async fn cancel_sync(&self, profile_id: Uuid) -> Result<(), MobileError> {
        let profile_id = profile_id_string(profile_id);
        native_unit(sync_cancel(&self.native, &profile_id))
    }
}

#[async_trait]
impl PlatformTransport for AndroidPlatform {
    async fn execute(
        &self,
        request: &PlatformHttpRequest<'_>,
    ) -> Result<PlatformHttpResponse, MobileError> {
        validate_http_request(request)?;
        let native_request = NativeHttpRequest {
            profile_id: request.profile_id,
            origin: request.origin.as_str(),
            url: request.url.as_str(),
            method: match request.method {
                PlatformHttpMethod::Get => "GET",
                PlatformHttpMethod::Post => "POST",
            },
            headers: request.headers,
            body_base64: STANDARD.encode(request.body),
            maximum_response_bytes: request.maximum_response_bytes,
        };
        let request_json = serde_json::to_string(&native_request)?;
        let envelope = native_envelope(http_start(&self.native, &request_json))?;
        if envelope.status != "PENDING" {
            return Err(MobileError::Permanent);
        }
        let request_id = envelope.request_id.ok_or(MobileError::Permanent)?;
        validate_request_id(&request_id)?;
        let mut guard = NativeRequestGuard::new(&self.native, request_id, http_cancel);
        poll_http(&self.native, &mut guard, request.maximum_response_bytes).await
    }
}

async fn poll_http(
    native: &WisecrowPlatform,
    request: &mut NativeRequestGuard<'_>,
    maximum_bytes: u64,
) -> Result<PlatformHttpResponse, MobileError> {
    loop {
        let envelope = native_envelope(http_poll(native, &request.request_id))?;
        match envelope.status.as_str() {
            "PENDING" => tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await,
            "CANCELLED" => {
                request.complete();
                return Err(MobileError::Cancelled);
            }
            "READY" => {
                request.complete();
                return native_http_response(envelope, maximum_bytes);
            }
            _ => return Err(MobileError::Permanent),
        }
    }
}

fn native_http_response(
    envelope: NativeEnvelope,
    maximum_bytes: u64,
) -> Result<PlatformHttpResponse, MobileError> {
    let status = envelope.http_status.ok_or(MobileError::Permanent)?;
    if !(100..=599).contains(&status) {
        return Err(MobileError::Permanent);
    }
    let headers = envelope.headers.ok_or(MobileError::Permanent)?;
    validate_response_headers(&headers)?;
    let encoded = envelope.body_base64.ok_or(MobileError::Permanent)?;
    validate_encoded_size(&encoded, maximum_bytes)?;
    let body = STANDARD
        .decode(encoded)
        .map_err(|_| MobileError::Permanent)?;
    if u64::try_from(body.len()).map_err(|_| MobileError::Permanent)? > maximum_bytes {
        return Err(MobileError::Permanent);
    }
    Ok(PlatformHttpResponse {
        status,
        headers,
        body,
    })
}

fn validate_http_request(request: &PlatformHttpRequest<'_>) -> Result<(), MobileError> {
    if request.maximum_response_bytes == 0
        || request.maximum_response_bytes > MAX_HTTP_RESPONSE_BYTES
        || request.body.len() > MAX_HTTP_REQUEST_BYTES
        || request.headers.len() > MAX_HTTP_HEADERS
    {
        return Err(invalid_http_request());
    }
    if !valid_origin(request.origin) || !valid_request_url(request.origin, request.url) {
        return Err(invalid_http_request());
    }
    if matches!(request.method, PlatformHttpMethod::Get) && !request.body.is_empty() {
        return Err(invalid_http_request());
    }
    validate_request_headers(request.headers)
}

fn valid_origin(origin: &url::Url) -> bool {
    origin.scheme() == "https"
        && origin.username().is_empty()
        && origin.password().is_none()
        && origin.query().is_none()
        && origin.fragment().is_none()
        && origin.path().ends_with('/')
}

fn valid_request_url(origin: &url::Url, request: &url::Url) -> bool {
    request.scheme() == "https"
        && request.username().is_empty()
        && request.password().is_none()
        && request.fragment().is_none()
        && request.scheme() == origin.scheme()
        && request.host_str() == origin.host_str()
        && request.port_or_known_default() == origin.port_or_known_default()
        && request.path().starts_with(origin.path())
}

fn validate_request_headers(headers: &[(&str, &str)]) -> Result<(), MobileError> {
    let mut total = 0usize;
    for (name, value) in headers {
        if name.len() > MAX_HTTP_HEADER_FIELD_BYTES
            || value.len() > MAX_HTTP_HEADER_FIELD_BYTES
            || forbidden_request_header(name)
        {
            return Err(invalid_http_request());
        }
        total = total
            .checked_add(name.len())
            .and_then(|length| length.checked_add(value.len()))
            .ok_or_else(invalid_http_request)?;
    }
    if total > MAX_HTTP_HEADER_BYTES {
        return Err(invalid_http_request());
    }
    Ok(())
}

fn validate_response_headers(headers: &[(String, String)]) -> Result<(), MobileError> {
    if headers.len() > MAX_HTTP_HEADERS {
        return Err(MobileError::Permanent);
    }
    let mut total = 0usize;
    for (name, value) in headers {
        if name.len() > MAX_HTTP_HEADER_FIELD_BYTES || value.len() > MAX_HTTP_HEADER_FIELD_BYTES {
            return Err(MobileError::Permanent);
        }
        total = total
            .checked_add(name.len())
            .and_then(|length| length.checked_add(value.len()))
            .ok_or(MobileError::Permanent)?;
    }
    if total > MAX_HTTP_HEADER_BYTES {
        return Err(MobileError::Permanent);
    }
    Ok(())
}

fn forbidden_request_header(name: &str) -> bool {
    ["connection", "content-length", "host", "transfer-encoding"]
        .iter()
        .any(|forbidden| name.eq_ignore_ascii_case(forbidden))
}

fn invalid_http_request() -> MobileError {
    MobileError::InvalidInput(String::from("the native HTTP request is invalid"))
}

const POLL_INTERVAL_MS: u64 = 50;
const UUID_TEXT_BYTES: usize = 36;
const MAX_NAME_CHARS: usize = 255;
const MAX_CERTIFICATE_BYTES: u64 = 65_536;
const MAX_PDF_BYTES: u64 = 67_108_864;
const SHA256_HEX_BYTES: usize = 64;
const MAX_HTTP_RESPONSE_BYTES: u64 = 67_108_864;
const MAX_HTTP_REQUEST_BYTES: usize = 16_777_216;
const MAX_HTTP_HEADERS: usize = 64;
const MAX_HTTP_HEADER_BYTES: usize = 32_768;
const MAX_HTTP_HEADER_FIELD_BYTES: usize = 8_192;
