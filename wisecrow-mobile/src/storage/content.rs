use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{de::DeserializeOwned, Serialize};
use sqlx::{FromRow, Sqlite, Transaction};
use wisecrow_dto::{
    CachedQuizDto, NbackBatchResponseDto, NbackSessionUploadDto, NbackUploadAckDto,
    NbackUploadStatusDto, MOBILE_PROTOCOL_VERSION,
};

use super::{
    models::{MediaEntry, MediaRegistration, MediaType},
    sqlite::{active_scope, active_scope_from_pool, StoreScope},
    SqliteStore,
};
use crate::application::{ContentRepository, MobileError};

const MAX_CONTENT_ITEMS: u16 = 500;
const MAX_ATTRIBUTION_BYTES: usize = 4_096;

#[derive(FromRow)]
struct MediaRow {
    translation_id: i32,
    media_type: String,
    file_name: String,
    byte_length: i64,
    attribution: Option<String>,
    last_accessed_at: DateTime<Utc>,
}

#[async_trait]
impl ContentRepository for SqliteStore {
    async fn save_nback(&self, session: &NbackSessionUploadDto) -> Result<(), MobileError> {
        validate_nback(session)?;
        let payload = canonical_json(session)?;
        let scope = active_scope_from_pool(&self.pool).await?;
        let existing = sqlx::query_scalar::<_, String>(
            "SELECT payload_json FROM nback_outbox
             WHERE profile_id = ? AND user_id = ? AND client_session_id = ?",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(session.client_session_id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(existing) = existing {
            return if existing == payload {
                Ok(())
            } else {
                Err(conflict(
                    "the N-Back session identifier has a different payload",
                ))
            };
        }
        insert_nback(&self.pool, scope, session, &payload).await
    }

    async fn pending_nback(&self, limit: u16) -> Result<Vec<NbackSessionUploadDto>, MobileError> {
        validate_limit(limit)?;
        let scope = active_scope_from_pool(&self.pool).await?;
        let payloads = sqlx::query_scalar::<_, String>(
            "SELECT payload_json FROM nback_outbox
             WHERE profile_id = ? AND user_id = ? AND status = 'pending'
             ORDER BY client_session_id LIMIT ?",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        payloads
            .into_iter()
            .map(|payload| serde_json::from_str(&payload).map_err(MobileError::from))
            .collect()
    }

    async fn apply_nback_response(
        &self,
        response: &NbackBatchResponseDto,
    ) -> Result<(), MobileError> {
        validate_nback_response(response)?;
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        for acknowledgement in &response.acknowledgements {
            apply_nback_acknowledgement(&mut transaction, scope, acknowledgement).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn save_quiz(&self, quiz: &CachedQuizDto) -> Result<(), MobileError> {
        validate_quiz(quiz)?;
        let cache_key = canonical_json(&quiz.key)?;
        let payload = canonical_json(quiz)?;
        let scope = active_scope_from_pool(&self.pool).await?;
        sqlx::query(
            "INSERT INTO cached_quizzes
                 (profile_id, user_id, cache_key, label, payload_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(profile_id, user_id, cache_key) DO UPDATE SET
                 label = excluded.label, payload_json = excluded.payload_json,
                 created_at = excluded.created_at",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(cache_key)
        .bind(&quiz.label)
        .bind(payload)
        .bind(quiz.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn cached_quizzes(&self) -> Result<Vec<CachedQuizDto>, MobileError> {
        let scope = active_scope_from_pool(&self.pool).await?;
        let payloads = sqlx::query_scalar::<_, String>(
            "SELECT payload_json FROM cached_quizzes
             WHERE profile_id = ? AND user_id = ? ORDER BY created_at DESC, cache_key",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .fetch_all(&self.pool)
        .await?;
        payloads
            .into_iter()
            .map(|payload| serde_json::from_str(&payload).map_err(MobileError::from))
            .collect()
    }

    async fn register_media(
        &self,
        media_root: &Path,
        registration: &MediaRegistration,
    ) -> Result<(), MobileError> {
        validate_media_registration(registration)?;
        let root = canonical_root(media_root)?;
        let path = canonical_media_path(&root, &registration.file_name)?;
        let actual_bytes = std::fs::metadata(path)?.len();
        if actual_bytes != registration.byte_length {
            return Err(invalid_input(
                "media byte length does not match the local file",
            ));
        }
        let scope = active_scope_from_pool(&self.pool).await?;
        require_translation(&self.pool, scope, registration.translation_id).await?;
        upsert_media(&self.pool, scope, registration).await
    }

    async fn media(
        &self,
        media_root: &Path,
        translation_id: i32,
        media_type: MediaType,
        accessed_at: DateTime<Utc>,
    ) -> Result<Option<MediaEntry>, MobileError> {
        validate_translation_id(translation_id)?;
        let root = canonical_root(media_root)?;
        let scope = active_scope_from_pool(&self.pool).await?;
        let row = media_row(&self.pool, scope, translation_id, media_type).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let entry = media_entry(&root, row)?;
        update_media_access(&self.pool, scope, translation_id, media_type, accessed_at).await?;
        Ok(Some(MediaEntry {
            last_accessed_at: accessed_at,
            ..entry
        }))
    }

    async fn media_lru(&self, media_root: &Path) -> Result<Vec<MediaEntry>, MobileError> {
        let root = canonical_root(media_root)?;
        let scope = active_scope_from_pool(&self.pool).await?;
        let rows = all_media_rows(&self.pool, scope).await?;
        rows.into_iter()
            .map(|row| media_entry(&root, row))
            .collect()
    }

    async fn eviction_candidates(
        &self,
        media_root: &Path,
        bytes_to_free: u64,
    ) -> Result<Vec<MediaEntry>, MobileError> {
        if bytes_to_free == 0 {
            return Ok(Vec::new());
        }
        let entries = self.media_lru(media_root).await?;
        let mut selected = Vec::new();
        let mut selected_bytes = 0u64;
        for entry in entries {
            selected_bytes = selected_bytes.saturating_add(entry.byte_length);
            selected.push(entry);
            if selected_bytes >= bytes_to_free {
                break;
            }
        }
        Ok(selected)
    }

    async fn confirm_media_deleted(
        &self,
        translation_id: i32,
        media_type: MediaType,
    ) -> Result<(), MobileError> {
        validate_translation_id(translation_id)?;
        let scope = active_scope_from_pool(&self.pool).await?;
        sqlx::query(
            "DELETE FROM media_cache
             WHERE profile_id = ? AND user_id = ? AND translation_id = ? AND media_type = ?",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(translation_id)
        .bind(media_type_value(media_type))
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

async fn insert_nback(
    pool: &sqlx::SqlitePool,
    scope: super::sqlite::StoreScope,
    session: &NbackSessionUploadDto,
    payload: &str,
) -> Result<(), MobileError> {
    sqlx::query(
        "INSERT INTO nback_outbox
             (profile_id, user_id, client_session_id, payload_json, status)
         VALUES (?, ?, ?, ?, 'pending')",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(session.client_session_id)
    .bind(payload)
    .execute(pool)
    .await?;
    Ok(())
}

async fn apply_nback_acknowledgement(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: StoreScope,
    acknowledgement: &NbackUploadAckDto,
) -> Result<(), MobileError> {
    let (status, reason) = match &acknowledgement.status {
        NbackUploadStatusDto::Applied | NbackUploadStatusDto::AlreadyApplied => ("applied", None),
        NbackUploadStatusDto::Rejected { reason } => {
            validate_rejection_reason(reason)?;
            ("rejected", Some(reason.as_str()))
        }
    };
    let result = sqlx::query(
        "UPDATE nback_outbox SET status = ?, rejection_reason = ?
         WHERE profile_id = ? AND user_id = ? AND client_session_id = ?",
    )
    .bind(status)
    .bind(reason)
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(acknowledgement.client_session_id)
    .execute(&mut **transaction)
    .await?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(conflict("the N-Back acknowledgement is unknown"))
    }
}

async fn require_translation(
    pool: &sqlx::SqlitePool,
    scope: super::sqlite::StoreScope,
    translation_id: i32,
) -> Result<(), MobileError> {
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM translations
         WHERE profile_id = ? AND user_id = ? AND translation_id = ?)",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(translation_id)
    .fetch_one(pool)
    .await?;
    if exists {
        Ok(())
    } else {
        Err(conflict("the media translation does not exist offline"))
    }
}

async fn upsert_media(
    pool: &sqlx::SqlitePool,
    scope: super::sqlite::StoreScope,
    registration: &MediaRegistration,
) -> Result<(), MobileError> {
    let byte_length = i64::try_from(registration.byte_length)
        .map_err(|_| invalid_input("media byte length is invalid"))?;
    sqlx::query(
        "INSERT INTO media_cache
             (profile_id, user_id, translation_id, media_type, file_name,
              byte_length, attribution, last_accessed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(profile_id, user_id, translation_id, media_type) DO UPDATE SET
             file_name = excluded.file_name, byte_length = excluded.byte_length,
             attribution = excluded.attribution, last_accessed_at = excluded.last_accessed_at",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(registration.translation_id)
    .bind(media_type_value(registration.media_type))
    .bind(&registration.file_name)
    .bind(byte_length)
    .bind(&registration.attribution)
    .bind(registration.last_accessed_at)
    .execute(pool)
    .await?;
    Ok(())
}

async fn media_row(
    pool: &sqlx::SqlitePool,
    scope: super::sqlite::StoreScope,
    translation_id: i32,
    media_type: MediaType,
) -> Result<Option<MediaRow>, MobileError> {
    Ok(sqlx::query_as::<_, MediaRow>(
        "SELECT translation_id, media_type, file_name, byte_length, attribution,
                last_accessed_at FROM media_cache
         WHERE profile_id = ? AND user_id = ? AND translation_id = ? AND media_type = ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(translation_id)
    .bind(media_type_value(media_type))
    .fetch_optional(pool)
    .await?)
}

async fn all_media_rows(
    pool: &sqlx::SqlitePool,
    scope: super::sqlite::StoreScope,
) -> Result<Vec<MediaRow>, MobileError> {
    Ok(sqlx::query_as::<_, MediaRow>(
        "SELECT translation_id, media_type, file_name, byte_length, attribution,
                last_accessed_at FROM media_cache
         WHERE profile_id = ? AND user_id = ?
         ORDER BY last_accessed_at, translation_id, media_type",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .fetch_all(pool)
    .await?)
}

async fn update_media_access(
    pool: &sqlx::SqlitePool,
    scope: super::sqlite::StoreScope,
    translation_id: i32,
    media_type: MediaType,
    accessed_at: DateTime<Utc>,
) -> Result<(), MobileError> {
    sqlx::query(
        "UPDATE media_cache SET last_accessed_at = ?
         WHERE profile_id = ? AND user_id = ? AND translation_id = ? AND media_type = ?",
    )
    .bind(accessed_at)
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(translation_id)
    .bind(media_type_value(media_type))
    .execute(pool)
    .await?;
    Ok(())
}

fn media_entry(root: &Path, row: MediaRow) -> Result<MediaEntry, MobileError> {
    Ok(MediaEntry {
        translation_id: row.translation_id,
        media_type: media_type_from_str(&row.media_type)?,
        path: canonical_media_path(root, &row.file_name)?,
        byte_length: u64::try_from(row.byte_length).map_err(|_| invalid_state())?,
        attribution: row.attribution,
        last_accessed_at: row.last_accessed_at,
    })
}

fn canonical_root(media_root: &Path) -> Result<PathBuf, MobileError> {
    let root = media_root.canonicalize()?;
    if root.is_dir() {
        Ok(root)
    } else {
        Err(invalid_input("media root is not a directory"))
    }
}

fn canonical_media_path(root: &Path, file_name: &str) -> Result<PathBuf, MobileError> {
    validate_file_name(file_name)?;
    let path = root.join(file_name).canonicalize()?;
    if path.starts_with(root) {
        Ok(path)
    } else {
        Err(invalid_input("media file is outside the app-private root"))
    }
}

fn validate_media_registration(registration: &MediaRegistration) -> Result<(), MobileError> {
    validate_translation_id(registration.translation_id)?;
    validate_file_name(&registration.file_name)?;
    if registration.byte_length == 0 {
        return Err(invalid_input("media byte length is invalid"));
    }
    if let Some(attribution) = &registration.attribution {
        let invalid = attribution.len() > MAX_ATTRIBUTION_BYTES
            || attribution.contains('\0')
            || attribution.chars().any(|character| character.is_control());
        if invalid {
            return Err(invalid_input("media attribution is invalid"));
        }
    }
    Ok(())
}

fn validate_file_name(file_name: &str) -> Result<(), MobileError> {
    let path = Path::new(file_name);
    let mut components = path.components();
    let valid_component = matches!(components.next(), Some(Component::Normal(_)));
    let valid = !file_name.is_empty()
        && file_name.len() <= 255
        && valid_component
        && components.next().is_none()
        && !file_name.contains('\0');
    if valid {
        Ok(())
    } else {
        Err(invalid_input("media file name is invalid"))
    }
}

fn validate_nback(session: &NbackSessionUploadDto) -> Result<(), MobileError> {
    let valid = session.n_level > 0
        && session.interval_ms > 0
        && !session.vocabulary_translation_ids.is_empty()
        && session
            .vocabulary_translation_ids
            .iter()
            .all(|translation_id| *translation_id > 0)
        && session.completed_at >= session.started_at;
    if valid {
        Ok(())
    } else {
        Err(invalid_input("completed N-Back session is invalid"))
    }
}

fn validate_nback_response(response: &NbackBatchResponseDto) -> Result<(), MobileError> {
    if response.protocol_version == MOBILE_PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(MobileError::UnsupportedProtocol {
            required: MOBILE_PROTOCOL_VERSION,
            actual: response.protocol_version,
        })
    }
}

fn validate_rejection_reason(reason: &str) -> Result<(), MobileError> {
    if reason.is_empty() || reason.len() > 1_024 || reason.chars().any(char::is_control) {
        Err(invalid_input("N-Back rejection reason is invalid"))
    } else {
        Ok(())
    }
}

fn validate_quiz(quiz: &CachedQuizDto) -> Result<(), MobileError> {
    let valid = quiz.key.item_count > 0
        && usize::from(quiz.key.item_count) == quiz.items.len()
        && !quiz.key.source_sha256.is_empty()
        && quiz.key.source_sha256.len() <= 128
        && !quiz.key.native_lang.is_empty()
        && !quiz.key.foreign_lang.is_empty()
        && !quiz.label.is_empty()
        && quiz.label.len() <= 1_024;
    if valid {
        Ok(())
    } else {
        Err(invalid_input("cached quiz is invalid"))
    }
}

fn validate_translation_id(translation_id: i32) -> Result<(), MobileError> {
    if translation_id > 0 {
        Ok(())
    } else {
        Err(invalid_input("translation identifier is invalid"))
    }
}

fn validate_limit(limit: u16) -> Result<(), MobileError> {
    if limit == 0 || limit > MAX_CONTENT_ITEMS {
        Err(invalid_input("content query limit is invalid"))
    } else {
        Ok(())
    }
}

fn canonical_json<T>(value: &T) -> Result<String, MobileError>
where
    T: Serialize + DeserializeOwned + PartialEq,
{
    let payload = serde_json::to_string(value)?;
    let decoded: T = serde_json::from_str(&payload)?;
    if decoded == *value {
        Ok(payload)
    } else {
        Err(invalid_state())
    }
}

fn media_type_value(media_type: MediaType) -> &'static str {
    match media_type {
        MediaType::Audio => "audio",
        MediaType::Image => "image",
    }
}

fn media_type_from_str(value: &str) -> Result<MediaType, MobileError> {
    match value {
        "audio" => Ok(MediaType::Audio),
        "image" => Ok(MediaType::Image),
        _ => Err(invalid_state()),
    }
}

fn invalid_input(message: &str) -> MobileError {
    MobileError::InvalidInput(String::from(message))
}

fn conflict(message: &str) -> MobileError {
    MobileError::Conflict(String::from(message))
}

fn invalid_state() -> MobileError {
    conflict("stored offline content is invalid")
}
