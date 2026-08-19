use std::{path::Path, sync::Arc};

use chrono::{DateTime, Utc};

use async_trait::async_trait;
use uuid::Uuid;
use wisecrow_dto::{
    CachedQuizDto, CardChangePageDto, CardChangeRequestDto, CardSnapshotDto, CorpusChangePageDto,
    CorpusChangeRequestDto, CorpusPageDto, CorpusSnapshotRequestDto, CorpusTranslationDto,
    DeviceRegistrationRequestDto, LanguageInfo, LanguagePairDto, MobileCapabilitiesDto,
    MobileSessionDto, NbackBatchRequestDto, NbackBatchResponseDto, NbackSessionUploadDto,
    RegisteredDeviceDto, ReviewBatchRequestDto, ReviewBatchResponseDto, ReviewEventDto, UserDto,
};

use super::error::MobileError;
use crate::storage::models::{
    CorpusEstimate, LocalAnswer, LocalSession, LocalSessionRequest, MediaEntry, MediaRegistration,
    MediaType, PairStatus, PairSyncState, PickedFile, Profile, ProfileIdentity, SyncErrorKind,
    SyncPhase,
};

#[async_trait]
pub trait ProfileRepository: Send + Sync {
    async fn active_profile(&self) -> Result<Option<Profile>, MobileError>;
    async fn active_identity(&self) -> Result<Option<ProfileIdentity>, MobileError>;
    async fn save_profile(&self, profile: &Profile) -> Result<(), MobileError>;
    async fn activate_profile(&self, profile_id: Uuid) -> Result<(), MobileError>;
    async fn profile_identity(
        &self,
        profile_id: Uuid,
        user_id: i32,
    ) -> Result<Option<ProfileIdentity>, MobileError>;
    async fn save_profile_identity(&self, identity: &ProfileIdentity) -> Result<(), MobileError>;
}

#[async_trait]
pub trait CorpusRepository: Send + Sync {
    async fn save_languages(&self, languages: &[LanguageInfo]) -> Result<(), MobileError>;
    async fn begin_snapshot(
        &self,
        pair: &LanguagePairDto,
        snapshot_watermark: i64,
        estimated_bytes: Option<u64>,
    ) -> Result<(), MobileError>;
    async fn apply_snapshot_page(&self, page: &CorpusPageDto) -> Result<(), MobileError>;
    async fn apply_change_page(&self, page: &CorpusChangePageDto) -> Result<(), MobileError>;
    async fn apply_card_page(&self, page: &CardChangePageDto) -> Result<(), MobileError>;
    async fn pair_status(&self, pair: &LanguagePairDto) -> Result<PairStatus, MobileError>;
    async fn corpus_estimate(&self, pair: &LanguagePairDto) -> Result<CorpusEstimate, MobileError>;
    async fn ranked_translations(
        &self,
        pair: &LanguagePairDto,
        limit: u16,
    ) -> Result<Vec<CorpusTranslationDto>, MobileError>;
    async fn translation(
        &self,
        pair: &LanguagePairDto,
        translation_id: i32,
    ) -> Result<Option<CorpusTranslationDto>, MobileError>;
    async fn sync_pairs(&self) -> Result<Vec<PairSyncState>, MobileError>;
    async fn card_cursor(&self) -> Result<i64, MobileError>;
    async fn sync_phase(&self) -> Result<SyncPhase, MobileError>;
    async fn advance_sync_phase(
        &self,
        expected: SyncPhase,
        next: SyncPhase,
    ) -> Result<(), MobileError>;
    async fn record_sync_success(&self, completed_at: DateTime<Utc>) -> Result<(), MobileError>;
    async fn record_sync_error(
        &self,
        kind: SyncErrorKind,
        failed_at: DateTime<Utc>,
    ) -> Result<(), MobileError>;
}

#[async_trait]
pub trait LearningRepository: Send + Sync {
    async fn create_session(
        &self,
        request: &LocalSessionRequest,
    ) -> Result<LocalSession, MobileError>;
    async fn answer(&self, answer: &LocalAnswer) -> Result<CardSnapshotDto, MobileError>;
    async fn pending_reviews(&self, limit: u16) -> Result<Vec<ReviewEventDto>, MobileError>;
    async fn apply_review_response(
        &self,
        response: &ReviewBatchResponseDto,
    ) -> Result<(), MobileError>;
}

#[async_trait]
pub trait ContentRepository: Send + Sync {
    async fn save_nback(&self, session: &NbackSessionUploadDto) -> Result<(), MobileError>;
    async fn pending_nback(&self, limit: u16) -> Result<Vec<NbackSessionUploadDto>, MobileError>;
    async fn apply_nback_response(
        &self,
        response: &NbackBatchResponseDto,
    ) -> Result<(), MobileError>;
    async fn save_quiz(&self, quiz: &CachedQuizDto) -> Result<(), MobileError>;
    async fn cached_quizzes(&self) -> Result<Vec<CachedQuizDto>, MobileError>;
    async fn register_media(
        &self,
        media_root: &Path,
        registration: &MediaRegistration,
    ) -> Result<(), MobileError>;
    async fn media(
        &self,
        media_root: &Path,
        translation_id: i32,
        media_type: MediaType,
        accessed_at: DateTime<Utc>,
    ) -> Result<Option<MediaEntry>, MobileError>;
    async fn media_lru(&self, media_root: &Path) -> Result<Vec<MediaEntry>, MobileError>;
    async fn eviction_candidates(
        &self,
        media_root: &Path,
        bytes_to_free: u64,
    ) -> Result<Vec<MediaEntry>, MobileError>;
    async fn confirm_media_deleted(
        &self,
        translation_id: i32,
        media_type: MediaType,
    ) -> Result<(), MobileError>;
}

pub trait LocalStore:
    ProfileRepository + CorpusRepository + LearningRepository + ContentRepository
{
}

impl<T> LocalStore for T where
    T: ProfileRepository + CorpusRepository + LearningRepository + ContentRepository
{
}

#[async_trait]
pub trait MobileApi: Send + Sync {
    async fn capabilities(&self) -> Result<MobileCapabilitiesDto, MobileError>;
    async fn login(&self, email: &str, password: &str) -> Result<MobileSessionDto, MobileError>;
    async fn me(&self) -> Result<UserDto, MobileError>;
    async fn languages(&self) -> Result<Vec<LanguageInfo>, MobileError>;
    async fn logout(&self) -> Result<(), MobileError>;
    async fn register_device(
        &self,
        request: &DeviceRegistrationRequestDto,
    ) -> Result<RegisteredDeviceDto, MobileError>;
    async fn corpus_snapshot(
        &self,
        request: &CorpusSnapshotRequestDto,
    ) -> Result<CorpusPageDto, MobileError>;
    async fn corpus_changes(
        &self,
        request: &CorpusChangeRequestDto,
    ) -> Result<CorpusChangePageDto, MobileError>;
    async fn card_changes(
        &self,
        request: &CardChangeRequestDto,
    ) -> Result<CardChangePageDto, MobileError>;
    async fn upload_reviews(
        &self,
        request: &ReviewBatchRequestDto,
    ) -> Result<ReviewBatchResponseDto, MobileError>;
    async fn upload_nback(
        &self,
        request: &NbackBatchRequestDto,
    ) -> Result<NbackBatchResponseDto, MobileError>;
}

pub trait ApiFactory: Send + Sync {
    fn create(&self, profile: &Profile) -> Result<Arc<dyn MobileApi>, MobileError>;
}

#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn load(&self, profile_id: Uuid) -> Result<Option<String>, MobileError>;
    async fn save(&self, profile_id: Uuid, token: &str) -> Result<(), MobileError>;
    async fn delete(&self, profile_id: Uuid) -> Result<(), MobileError>;
}

#[async_trait]
pub trait CertificateStore: Send + Sync {
    async fn load(&self, profile_id: Uuid) -> Result<Option<Vec<u8>>, MobileError>;
    async fn save(&self, profile_id: Uuid, certificate: &[u8]) -> Result<(), MobileError>;
    async fn delete(&self, profile_id: Uuid) -> Result<(), MobileError>;
}

#[async_trait]
pub trait FilePicker: Send + Sync {
    async fn pick_pdf(&self, maximum_bytes: u64) -> Result<Option<PickedFile>, MobileError>;
    async fn pick_certificate(&self, maximum_bytes: u64)
        -> Result<Option<PickedFile>, MobileError>;
}

#[async_trait]
pub trait BackgroundScheduler: Send + Sync {
    async fn schedule_sync(&self, profile_id: Uuid) -> Result<(), MobileError>;
    async fn cancel_sync(&self, profile_id: Uuid) -> Result<(), MobileError>;
}
