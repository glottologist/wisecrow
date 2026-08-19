use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use uuid::Uuid;
use wisecrow_dto::{
    CardSnapshotDto, CorpusTranslationDto, LanguagePairDto, ReviewRatingDto, UserDto,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: Uuid,
    pub origin: String,
    pub imported_ca_fingerprint: Option<String>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileIdentity {
    pub profile: Profile,
    pub user: UserDto,
    pub device_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairStatus {
    Absent,
    Downloading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusEstimate {
    pub translation_count: u64,
    pub text_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairSyncState {
    pub pair: LanguagePairDto,
    pub status: PairStatus,
    pub snapshot_watermark: Option<i64>,
    pub snapshot_after_id: i32,
    pub change_cursor: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncErrorKind {
    Retryable,
    Permanent,
    Authentication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncPhase {
    Idle,
    Reviews,
    Nback,
    Cards,
    Snapshots,
    Deltas,
    Finishing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocalSessionStatus {
    Active,
    Paused,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalSessionRequest {
    pub id: Uuid,
    pub pair: LanguagePairDto,
    pub deck_size: u16,
    pub speed_ms: u32,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalSession {
    pub id: Uuid,
    pub pair: LanguagePairDto,
    pub speed_ms: u32,
    pub current_index: u32,
    pub status: LocalSessionStatus,
    pub started_at: DateTime<Utc>,
    pub cards: Vec<LocalSessionCard>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalSessionCard {
    pub translation: CorpusTranslationDto,
    pub card: CardSnapshotDto,
    pub answered: bool,
    pub rating: Option<ReviewRatingDto>,
    pub answered_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalAnswer {
    pub session_id: Uuid,
    pub event_id: Uuid,
    pub device_id: Uuid,
    pub translation_id: i32,
    pub rating: ReviewRatingDto,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaType {
    Audio,
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRegistration {
    pub translation_id: i32,
    pub media_type: MediaType,
    pub file_name: String,
    pub byte_length: u64,
    pub attribution: Option<String>,
    pub last_accessed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaEntry {
    pub translation_id: i32,
    pub media_type: MediaType,
    pub path: PathBuf,
    pub byte_length: u64,
    pub attribution: Option<String>,
    pub last_accessed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickedFile {
    pub display_name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}
