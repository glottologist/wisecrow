//! Turning a served clip into a registered file.
//!
//! The store registers files that already exist under the media root and
//! nothing else writes there, so this is the one place bytes from the
//! server become a file: decoded within a bound, written beside its final
//! name and renamed into place, then registered; a registration that fails
//! takes the file with it.

use std::path::Path;

use base64::Engine as _;
use chrono::{DateTime, Utc};
use wisecrow_dto::{MediaOwnerDto, MobileMediaDto, MobileMediaTypeDto};

use crate::application::{ContentRepository, MobileError};
use crate::storage::models::{MediaEntry, MediaOwner, MediaRegistration, MediaType};

/// Largest payload a device will decode: the server's own audio bound.
pub const MAX_MEDIA_BYTES: usize = 10 * 1024 * 1024;

/// The device's owner for a served payload.
#[must_use]
pub fn owner_from_dto(owner: &MediaOwnerDto) -> MediaOwner {
    match owner {
        MediaOwnerDto::Translation { id } => MediaOwner::Translation(*id),
        MediaOwnerDto::Sentence { fingerprint } => MediaOwner::Sentence(fingerprint.clone()), // clone: the device keeps its own name
    }
}

const fn media_type_from_dto(media_type: MobileMediaTypeDto) -> MediaType {
    match media_type {
        MobileMediaTypeDto::Audio => MediaType::Audio,
        MobileMediaTypeDto::Image => MediaType::Image,
    }
}

/// The file name a payload is stored under: one per owner and medium, so a
/// refetch overwrites rather than accumulates.
#[must_use]
pub fn file_name_for(owner: &MediaOwner, media_type: MediaType) -> String {
    let extension = match media_type {
        MediaType::Audio => "mp3",
        MediaType::Image => "jpg",
    };
    format!("{}-{}.{extension}", owner.kind(), owner.key())
}

/// Writes and registers one served payload, returning its entry.
///
/// # Errors
///
/// Returns an input error for an empty, oversized or undecodable payload, a
/// filesystem error when the file cannot be written, and the store's error
/// when registration fails (the file is removed first).
pub async fn publish_media<S: ContentRepository + ?Sized>(
    store: &S,
    media_root: &Path,
    media: &MobileMediaDto,
    accessed_at: DateTime<Utc>,
) -> Result<MediaEntry, MobileError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&media.bytes_base64)
        .map_err(|_| MobileError::InvalidInput(String::from("media payload is not base64")))?;
    if bytes.is_empty() || bytes.len() > MAX_MEDIA_BYTES {
        return Err(MobileError::InvalidInput(String::from(
            "media payload size is invalid",
        )));
    }
    let owner = owner_from_dto(&media.owner);
    let media_type = media_type_from_dto(media.media_type);
    let file_name = file_name_for(&owner, media_type);
    let final_path = media_root.join(&file_name);
    write_atomic(&final_path, &bytes)?;

    let registration = MediaRegistration {
        owner: owner.clone(), // clone: the registration and the lookup both name the owner
        media_type,
        file_name,
        byte_length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        attribution: media.attribution.clone(), // clone: the registration owns its credit
        fingerprint: media.fingerprint.clone(), // clone: the registration owns its identity
        last_accessed_at: accessed_at,
    };
    if let Err(error) = store.register_media(media_root, &registration).await {
        let _ = std::fs::remove_file(&final_path);
        return Err(error);
    }
    store
        .media(media_root, &owner, media_type, accessed_at)
        .await?
        .ok_or_else(|| MobileError::Conflict(String::from("registered media is not readable")))
}

fn write_atomic(final_path: &Path, bytes: &[u8]) -> Result<(), MobileError> {
    let temp_path = final_path.with_extension("tmp");
    std::fs::write(&temp_path, bytes)?;
    if let Err(error) = std::fs::rename(&temp_path, final_path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error.into());
    }
    Ok(())
}
