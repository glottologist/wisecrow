use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock, Weak},
};

use chrono::Utc;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use wisecrow_dto::{
    CardChangeRequestDto, CorpusChangeRequestDto, CorpusSnapshotRequestDto, MobileCapabilitiesDto,
    MobileFeatureDto, NbackBatchRequestDto, NbackBatchResponseDto, ReviewBatchRequestDto,
    ReviewBatchResponseDto, MOBILE_PROTOCOL_VERSION,
};

use crate::{
    application::{LocalStore, MobileApi, MobileError},
    storage::models::{PairStatus, ProfileIdentity, SyncErrorKind, SyncPhase},
};

const MAX_LOCAL_PAGE: u16 = 500;
const MEDIA_PREFETCH_FOREGROUND: u16 = 25;
const MEDIA_PREFETCH_PERIODIC: u16 = 10;

static PROFILE_LOCKS: OnceLock<Mutex<HashMap<Uuid, Weak<AsyncMutex<()>>>>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncReason {
    Launch,
    ConnectivityRestored,
    Periodic,
    Manual,
    PackDownload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOutcome {
    Complete { media_prefetch_limit: u16 },
    AlreadyRunning,
    Cancelled,
    AuthenticationExpired,
    Retryable,
    PermanentFailure,
}

/// Restart-safe coordinator for outbox uploads and authoritative downloads.
pub struct SyncEngine {
    store: Arc<dyn LocalStore>,
    api: Arc<dyn MobileApi>,
    cancellation: Arc<CancellationToken>,
}

impl SyncEngine {
    #[must_use]
    pub fn new(
        store: impl Into<Arc<dyn LocalStore>>,
        api: impl Into<Arc<dyn MobileApi>>,
        cancellation: impl Into<Arc<CancellationToken>>,
    ) -> Self {
        Self {
            store: store.into(),
            api: api.into(),
            cancellation: cancellation.into(),
        }
    }

    pub async fn run(&self, reason: SyncReason) -> SyncOutcome {
        let identity = match self.store.active_identity().await {
            Ok(Some(identity)) => identity,
            Ok(None) | Err(MobileError::Authentication) => {
                return SyncOutcome::AuthenticationExpired;
            }
            Err(error) => return self.finish_error(&error).await,
        };
        let _permit = match acquire_profile_permit(identity.profile.id) {
            Ok(Some(permit)) => permit,
            Ok(None) => return SyncOutcome::AlreadyRunning,
            Err(error) => return self.finish_error(&error).await,
        };
        match self.run_locked(&identity, reason).await {
            Ok(outcome) => outcome,
            Err(error) => self.finish_error(&error).await,
        }
    }

    async fn run_locked(
        &self,
        identity: &ProfileIdentity,
        reason: SyncReason,
    ) -> Result<SyncOutcome, MobileError> {
        self.check_cancellation()?;
        let capabilities = self.validate_remote(identity).await?;
        let mut phase = self.store.sync_phase().await?;
        if phase == SyncPhase::Idle {
            self.refresh_languages().await?;
            self.advance_phase(SyncPhase::Idle, SyncPhase::Reviews)
                .await?;
            phase = SyncPhase::Reviews;
        }
        phase = self
            .run_upload_phases(phase, identity, &capabilities)
            .await?;
        phase = self.run_download_phases(phase, &capabilities).await?;
        if phase != SyncPhase::Finishing {
            return Err(invalid_state());
        }
        self.check_cancellation()?;
        self.store.record_sync_success(Utc::now()).await?;
        Ok(SyncOutcome::Complete {
            media_prefetch_limit: prefetch_limit(reason),
        })
    }

    async fn run_upload_phases(
        &self,
        mut phase: SyncPhase,
        identity: &ProfileIdentity,
        capabilities: &MobileCapabilitiesDto,
    ) -> Result<SyncPhase, MobileError> {
        if phase == SyncPhase::Reviews {
            self.upload_reviews(identity, capabilities.max_review_batch)
                .await?;
            self.advance_phase(SyncPhase::Reviews, SyncPhase::Nback)
                .await?;
            phase = SyncPhase::Nback;
        }
        if phase == SyncPhase::Nback {
            self.upload_nback(identity, capabilities.max_nback_batch)
                .await?;
            self.advance_phase(SyncPhase::Nback, SyncPhase::Cards)
                .await?;
            phase = SyncPhase::Cards;
        }
        Ok(phase)
    }

    async fn run_download_phases(
        &self,
        mut phase: SyncPhase,
        capabilities: &MobileCapabilitiesDto,
    ) -> Result<SyncPhase, MobileError> {
        if phase == SyncPhase::Cards {
            self.pull_cards(capabilities.max_snapshot_page).await?;
            self.advance_phase(SyncPhase::Cards, SyncPhase::Snapshots)
                .await?;
            phase = SyncPhase::Snapshots;
        }
        if phase == SyncPhase::Snapshots {
            self.pull_snapshots(capabilities.max_snapshot_page).await?;
            self.advance_phase(SyncPhase::Snapshots, SyncPhase::Deltas)
                .await?;
            phase = SyncPhase::Deltas;
        }
        if phase == SyncPhase::Deltas {
            self.pull_deltas(capabilities.max_snapshot_page).await?;
            self.advance_phase(SyncPhase::Deltas, SyncPhase::Finishing)
                .await?;
            phase = SyncPhase::Finishing;
        }
        Ok(phase)
    }

    async fn validate_remote(
        &self,
        identity: &ProfileIdentity,
    ) -> Result<MobileCapabilitiesDto, MobileError> {
        self.check_cancellation()?;
        let user = self.api.me().await?;
        if user.id != identity.user.id {
            return Err(MobileError::Authentication);
        }
        self.check_cancellation()?;
        let capabilities = self.api.capabilities().await?;
        validate_capabilities(&capabilities)?;
        Ok(capabilities)
    }

    async fn refresh_languages(&self) -> Result<(), MobileError> {
        self.check_cancellation()?;
        let languages = self.api.languages().await?;
        self.check_cancellation()?;
        self.store.save_languages(&languages).await
    }

    async fn upload_reviews(
        &self,
        identity: &ProfileIdentity,
        server_limit: u16,
    ) -> Result<(), MobileError> {
        let limit = bounded_limit(server_limit)?;
        loop {
            self.check_cancellation()?;
            let events = self.store.pending_reviews(limit).await?;
            if events.is_empty() {
                return Ok(());
            }
            let request = ReviewBatchRequestDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                device_id: identity.device_id,
                events,
            };
            self.check_cancellation()?;
            let response = self.api.upload_reviews(&request).await?;
            validate_review_acknowledgements(&request, &response)?;
            self.check_cancellation()?;
            self.store.apply_review_response(&response).await?;
        }
    }

    async fn upload_nback(
        &self,
        identity: &ProfileIdentity,
        server_limit: u16,
    ) -> Result<(), MobileError> {
        let limit = bounded_limit(server_limit)?;
        loop {
            self.check_cancellation()?;
            let sessions = self.store.pending_nback(limit).await?;
            if sessions.is_empty() {
                return Ok(());
            }
            let request = NbackBatchRequestDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                device_id: identity.device_id,
                sessions,
            };
            self.check_cancellation()?;
            let response = self.api.upload_nback(&request).await?;
            validate_nback_acknowledgements(&request, &response)?;
            self.check_cancellation()?;
            self.store.apply_nback_response(&response).await?;
        }
    }

    async fn pull_cards(&self, server_limit: u16) -> Result<(), MobileError> {
        let limit = bounded_limit(server_limit)?;
        loop {
            self.check_cancellation()?;
            let request = CardChangeRequestDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                cursor: self.store.card_cursor().await?,
                limit,
            };
            let page = self.api.card_changes(&request).await?;
            let has_more = page.has_more;
            self.check_cancellation()?;
            self.store.apply_card_page(&page).await?;
            if !has_more {
                return Ok(());
            }
        }
    }

    async fn pull_snapshots(&self, server_limit: u16) -> Result<(), MobileError> {
        let limit = bounded_limit(server_limit)?;
        for state in self.store.sync_pairs().await? {
            if state.status != PairStatus::Downloading {
                continue;
            }
            let request = CorpusSnapshotRequestDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                pair: state.pair,
                after_translation_id: state.snapshot_after_id,
                snapshot_watermark: state.snapshot_watermark,
                limit,
            };
            self.pull_snapshot_pair(request).await?;
        }
        Ok(())
    }

    async fn pull_snapshot_pair(
        &self,
        mut request: CorpusSnapshotRequestDto,
    ) -> Result<(), MobileError> {
        loop {
            self.check_cancellation()?;
            let page = self.api.corpus_snapshot(&request).await?;
            validate_snapshot_response(&request, &page)?;
            let has_more = page.has_more;
            self.check_cancellation()?;
            self.store.apply_snapshot_page(&page).await?;
            if !has_more {
                return Ok(());
            }
            request.pair = page.pair;
            request.after_translation_id = page.next_cursor;
            request.snapshot_watermark = Some(page.snapshot_watermark);
        }
    }

    async fn pull_deltas(&self, server_limit: u16) -> Result<(), MobileError> {
        let limit = bounded_limit(server_limit)?;
        for state in self.store.sync_pairs().await? {
            if state.status != PairStatus::Ready {
                continue;
            }
            let request = CorpusChangeRequestDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                pair: state.pair,
                cursor: state.change_cursor,
                limit,
            };
            self.pull_delta_pair(request).await?;
        }
        Ok(())
    }

    async fn pull_delta_pair(
        &self,
        mut request: CorpusChangeRequestDto,
    ) -> Result<(), MobileError> {
        loop {
            self.check_cancellation()?;
            let page = self.api.corpus_changes(&request).await?;
            validate_delta_response(&request, &page)?;
            let has_more = page.has_more;
            self.check_cancellation()?;
            self.store.apply_change_page(&page).await?;
            if !has_more {
                return Ok(());
            }
            request.pair = page.pair;
            request.cursor = page.next_cursor;
        }
    }

    async fn advance_phase(&self, expected: SyncPhase, next: SyncPhase) -> Result<(), MobileError> {
        self.check_cancellation()?;
        self.store.advance_sync_phase(expected, next).await
    }

    fn check_cancellation(&self) -> Result<(), MobileError> {
        if self.cancellation.is_cancelled() {
            Err(MobileError::Cancelled)
        } else {
            Ok(())
        }
    }

    async fn finish_error(&self, error: &MobileError) -> SyncOutcome {
        let (kind, outcome) = classify_error(error);
        let should_record = !matches!(error, MobileError::Cancelled);
        if should_record
            && self
                .store
                .record_sync_error(kind, Utc::now())
                .await
                .is_err()
        {
            return SyncOutcome::PermanentFailure;
        }
        outcome
    }
}

fn acquire_profile_permit(profile_id: Uuid) -> Result<Option<OwnedMutexGuard<()>>, MobileError> {
    let locks = PROFILE_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks.lock().map_err(|_| MobileError::Permanent)?;
    let lock = match locks.get(&profile_id).and_then(Weak::upgrade) {
        Some(lock) => lock,
        None => {
            let lock = Arc::new(AsyncMutex::new(()));
            locks.insert(profile_id, Arc::downgrade(&lock));
            lock
        }
    };
    drop(locks);
    match lock.try_lock_owned() {
        Ok(permit) => Ok(Some(permit)),
        Err(_) => Ok(None),
    }
}

fn validate_capabilities(capabilities: &MobileCapabilitiesDto) -> Result<(), MobileError> {
    if capabilities.protocol_version != MOBILE_PROTOCOL_VERSION {
        return Err(MobileError::UnsupportedProtocol {
            required: MOBILE_PROTOCOL_VERSION,
            actual: capabilities.protocol_version,
        });
    }
    let required = [
        MobileFeatureDto::CorpusSync,
        MobileFeatureDto::CardSync,
        MobileFeatureDto::ReviewUpload,
        MobileFeatureDto::NbackUpload,
    ];
    if required
        .iter()
        .all(|feature| capabilities.supported_features.contains(feature))
    {
        Ok(())
    } else {
        Err(MobileError::Permanent)
    }
}

fn validate_review_acknowledgements(
    request: &ReviewBatchRequestDto,
    response: &ReviewBatchResponseDto,
) -> Result<(), MobileError> {
    if response.protocol_version != MOBILE_PROTOCOL_VERSION
        || response.acknowledgements.len() != request.events.len()
    {
        return Err(MobileError::Permanent);
    }
    let requested: HashSet<Uuid> = request.events.iter().map(|event| event.event_id).collect();
    let acknowledged: HashSet<Uuid> = response
        .acknowledgements
        .iter()
        .map(|acknowledgement| acknowledgement.event_id)
        .collect();
    if requested == acknowledged && acknowledged.len() == response.acknowledgements.len() {
        Ok(())
    } else {
        Err(MobileError::Permanent)
    }
}

fn validate_nback_acknowledgements(
    request: &NbackBatchRequestDto,
    response: &NbackBatchResponseDto,
) -> Result<(), MobileError> {
    if response.protocol_version != MOBILE_PROTOCOL_VERSION
        || response.acknowledgements.len() != request.sessions.len()
    {
        return Err(MobileError::Permanent);
    }
    let requested: HashSet<Uuid> = request
        .sessions
        .iter()
        .map(|session| session.client_session_id)
        .collect();
    let acknowledged: HashSet<Uuid> = response
        .acknowledgements
        .iter()
        .map(|acknowledgement| acknowledgement.client_session_id)
        .collect();
    if requested == acknowledged && acknowledged.len() == response.acknowledgements.len() {
        Ok(())
    } else {
        Err(MobileError::Permanent)
    }
}

fn validate_snapshot_response(
    request: &CorpusSnapshotRequestDto,
    page: &wisecrow_dto::CorpusPageDto,
) -> Result<(), MobileError> {
    if page.protocol_version == MOBILE_PROTOCOL_VERSION
        && page.pair == request.pair
        && request.snapshot_watermark == Some(page.snapshot_watermark)
    {
        Ok(())
    } else {
        Err(MobileError::Permanent)
    }
}

fn validate_delta_response(
    request: &CorpusChangeRequestDto,
    page: &wisecrow_dto::CorpusChangePageDto,
) -> Result<(), MobileError> {
    if page.protocol_version == MOBILE_PROTOCOL_VERSION && page.pair == request.pair {
        Ok(())
    } else {
        Err(MobileError::Permanent)
    }
}

fn bounded_limit(server_limit: u16) -> Result<u16, MobileError> {
    if server_limit == 0 {
        Err(MobileError::Permanent)
    } else {
        Ok(server_limit.min(MAX_LOCAL_PAGE))
    }
}

fn prefetch_limit(reason: SyncReason) -> u16 {
    match reason {
        SyncReason::Periodic => MEDIA_PREFETCH_PERIODIC,
        SyncReason::Launch
        | SyncReason::ConnectivityRestored
        | SyncReason::Manual
        | SyncReason::PackDownload => MEDIA_PREFETCH_FOREGROUND,
    }
}

fn classify_error(error: &MobileError) -> (SyncErrorKind, SyncOutcome) {
    match error {
        MobileError::Cancelled => (SyncErrorKind::Retryable, SyncOutcome::Cancelled),
        MobileError::Retryable => (SyncErrorKind::Retryable, SyncOutcome::Retryable),
        MobileError::Authentication | MobileError::Credentials | MobileError::DeviceRevoked => (
            SyncErrorKind::Authentication,
            SyncOutcome::AuthenticationExpired,
        ),
        _ => (SyncErrorKind::Permanent, SyncOutcome::PermanentFailure),
    }
}

fn invalid_state() -> MobileError {
    MobileError::Conflict(String::from("local synchronization phase is invalid"))
}
