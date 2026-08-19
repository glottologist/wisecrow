use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{CardStatusDto, DnbSessionResultsDto, QuizItemDto};

/// Mobile protocol version supported by this build.
pub const MOBILE_PROTOCOL_VERSION: u16 = 1;

/// One independently negotiable mobile capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MobileFeatureDto {
    CorpusSync,
    CardSync,
    ReviewUpload,
    NbackUpload,
    QuizCache,
}

/// Public server capabilities needed during mobile onboarding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MobileCapabilitiesDto {
    pub protocol_version: u16,
    pub supported_features: Vec<MobileFeatureDto>,
    pub max_snapshot_page: u16,
    pub max_review_batch: u16,
    pub max_nback_batch: u16,
    pub server_version: String,
}

/// Stable category for a protocol-level failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolErrorKindDto {
    UnsupportedVersion,
    InvalidRequest,
    Conflict,
}

/// Sanitized protocol error returned to a mobile client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolErrorDto {
    pub supported_protocol_version: u16,
    pub kind: ProtocolErrorKindDto,
    pub message: String,
}

/// Registers one stable app installation for the authenticated account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRegistrationRequestDto {
    pub protocol_version: u16,
    pub device_id: Uuid,
    pub display_name: String,
}

/// Authoritative server record for a registered device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredDeviceDto {
    pub device_id: Uuid,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// Native/foreign language pair identifying one offline corpus pack.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LanguagePairDto {
    pub native_lang: String,
    pub foreign_lang: String,
}

/// One translation in a corpus snapshot or change payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusTranslationDto {
    pub translation_id: i32,
    pub from_phrase: String,
    pub to_phrase: String,
    pub frequency: i32,
    pub is_phrase: bool,
}

/// Requests one resumable page from a stable corpus snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusSnapshotRequestDto {
    pub protocol_version: u16,
    pub pair: LanguagePairDto,
    pub after_translation_id: i32,
    pub snapshot_watermark: Option<i64>,
    pub limit: u16,
}

/// One page of a stable corpus snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusPageDto {
    pub protocol_version: u16,
    pub pair: LanguagePairDto,
    pub translations: Vec<CorpusTranslationDto>,
    pub next_cursor: i32,
    pub has_more: bool,
    pub snapshot_watermark: i64,
}

/// Requests corpus changes after a durable sequence cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusChangeRequestDto {
    pub protocol_version: u16,
    pub pair: LanguagePairDto,
    pub cursor: i64,
    pub limit: u16,
}

/// Operation represented by one corpus change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CorpusChangeKindDto {
    Upsert,
    Delete,
}

/// One corpus upsert or tombstone from the server change ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusChangeDto {
    pub sequence: i64,
    pub translation_id: i32,
    pub kind: CorpusChangeKindDto,
    pub translation: Option<CorpusTranslationDto>,
    pub changed_at: DateTime<Utc>,
}

/// One page of corpus changes for an exact language pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusChangePageDto {
    pub protocol_version: u16,
    pub pair: LanguagePairDto,
    pub changes: Vec<CorpusChangeDto>,
    pub next_cursor: i64,
    pub has_more: bool,
    pub change_watermark: i64,
}

/// Rating stored in an immutable review event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewRatingDto {
    Again,
    Hard,
    Good,
    Easy,
}

impl TryFrom<u8> for ReviewRatingDto {
    type Error = ProtocolErrorDto;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Again),
            2 => Ok(Self::Hard),
            3 => Ok(Self::Good),
            4 => Ok(Self::Easy),
            _ => Err(ProtocolErrorDto {
                supported_protocol_version: MOBILE_PROTOCOL_VERSION,
                kind: ProtocolErrorKindDto::InvalidRequest,
                message: "review rating must be in 1..=4".into(),
            }),
        }
    }
}

/// Immutable review event created by one device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewEventDto {
    pub event_id: Uuid,
    pub translation_id: i32,
    pub rating: ReviewRatingDto,
    pub occurred_at: DateTime<Utc>,
}

/// Batched review upload from one registered device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewBatchRequestDto {
    pub protocol_version: u16,
    pub device_id: Uuid,
    pub events: Vec<ReviewEventDto>,
}

/// Per-event review upload outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewEventStatusDto {
    Applied,
    AlreadyApplied,
    Rejected { reason: String },
}

/// Acknowledgement for one immutable review event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewEventAckDto {
    pub event_id: Uuid,
    pub status: ReviewEventStatusDto,
}

/// Authoritative persisted FSRS state for one translation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardSnapshotDto {
    pub translation_id: i32,
    pub stability: f64,
    pub difficulty: f64,
    pub elapsed_days: i64,
    pub scheduled_days: i64,
    pub reps: i32,
    pub lapses: i32,
    pub state: CardStatusDto,
    pub last_review: Option<DateTime<Utc>>,
    pub due: DateTime<Utc>,
    pub server_cursor: i64,
}

/// Batched review result with per-item status and authoritative card state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewBatchResponseDto {
    pub protocol_version: u16,
    pub acknowledgements: Vec<ReviewEventAckDto>,
    pub cards: Vec<CardSnapshotDto>,
}

/// Requests authoritative card changes after a durable cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardChangeRequestDto {
    pub protocol_version: u16,
    pub cursor: i64,
    pub limit: u16,
}

/// One authoritative card upsert or tombstone.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CardChangeDto {
    Upsert {
        sequence: i64,
        card: CardSnapshotDto,
    },
    Delete {
        sequence: i64,
        translation_id: i32,
    },
}

/// One page from the authenticated user's card change stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CardChangePageDto {
    pub protocol_version: u16,
    pub changes: Vec<CardChangeDto>,
    pub next_cursor: i64,
    pub has_more: bool,
    pub change_watermark: i64,
}

/// Shared presentation mode for an uploaded N-Back session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NbackModeDto {
    AudioWritten,
    WordTranslation,
    AudioImage,
}

/// Button state and duration recorded for one ordered N-Back trial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NbackTrialResponseDto {
    pub trial_number: u32,
    pub audio_response: Option<bool>,
    pub visual_response: Option<bool>,
    pub response_time_ms: u32,
}

/// Complete deterministic N-Back session awaiting upload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NbackSessionUploadDto {
    pub client_session_id: Uuid,
    pub pair: LanguagePairDto,
    pub mode: NbackModeDto,
    pub n_level: u8,
    pub interval_ms: u32,
    pub seed: u64,
    pub vocabulary_translation_ids: Vec<i32>,
    pub responses: Vec<NbackTrialResponseDto>,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
}

/// Batched completed N-Back uploads from one registered device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NbackBatchRequestDto {
    pub protocol_version: u16,
    pub device_id: Uuid,
    pub sessions: Vec<NbackSessionUploadDto>,
}

/// Per-session N-Back upload outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NbackUploadStatusDto {
    Applied,
    AlreadyApplied,
    Rejected { reason: String },
}

/// Acknowledgement and server-computed result for one N-Back upload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NbackUploadAckDto {
    pub client_session_id: Uuid,
    pub status: NbackUploadStatusDto,
    pub result: Option<DnbSessionResultsDto>,
}

/// Result of uploading a batch of completed N-Back sessions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NbackBatchResponseDto {
    pub protocol_version: u16,
    pub acknowledgements: Vec<NbackUploadAckDto>,
}

/// Quiz generation shape participating in a cache key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum QuizKindDto {
    Cloze,
    MultipleChoice,
    Mixed,
}

/// Stable source and generation parameters for one cached quiz.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct QuizCacheKeyDto {
    pub source_sha256: String,
    pub native_lang: String,
    pub foreign_lang: String,
    pub kind: QuizKindDto,
    pub cefr_level: Option<String>,
    pub item_count: u16,
}

/// Server-generated quiz content retained for offline attempts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedQuizDto {
    pub key: QuizCacheKeyDto,
    pub label: String,
    pub items: Vec<QuizItemDto>,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde::{de::DeserializeOwned, Serialize};
    use std::fmt::Debug;

    fn assert_json_roundtrip<T>(value: &T)
    where
        T: Serialize + DeserializeOwned + PartialEq + Debug,
    {
        let json = serde_json::to_vec(value).expect("serialize");
        let decoded: T = serde_json::from_slice(&json).expect("deserialize");
        assert_eq!(&decoded, value);
    }

    fn pair() -> LanguagePairDto {
        LanguagePairDto {
            native_lang: "en".into(),
            foreign_lang: "de".into(),
        }
    }

    fn translation(translation_id: i32) -> CorpusTranslationDto {
        CorpusTranslationDto {
            translation_id,
            from_phrase: "eins".into(),
            to_phrase: "one".into(),
            frequency: 7,
            is_phrase: false,
        }
    }

    fn card(translation_id: i32, at: DateTime<Utc>, cursor: i64) -> CardSnapshotDto {
        CardSnapshotDto {
            translation_id,
            stability: 1.5,
            difficulty: 4.0,
            elapsed_days: 1,
            scheduled_days: 3,
            reps: 2,
            lapses: 0,
            state: CardStatusDto::Review,
            last_review: Some(at),
            due: at,
            server_cursor: cursor,
        }
    }

    proptest! {
        #[test]
        fn review_batch_json_roundtrips(
            translation_id in 1i32..i32::MAX,
            rating in 1u8..=4,
            timestamp in 1_600_000_000i64..2_000_000_000,
        ) {
            let occurred_at = chrono::DateTime::from_timestamp(timestamp, 0)
                .expect("generated timestamp");
            let request = ReviewBatchRequestDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                device_id: uuid::Uuid::from_u128(1),
                events: vec![ReviewEventDto {
                    event_id: uuid::Uuid::from_u128(2),
                    translation_id,
                    rating: ReviewRatingDto::try_from(rating).expect("generated rating"),
                    occurred_at,
                }],
            };
            assert_json_roundtrip(&request);
        }

        #[test]
        fn capability_and_device_json_roundtrip(timestamp in 1_600_000_000i64..2_000_000_000) {
            let at = DateTime::from_timestamp(timestamp, 0).expect("generated timestamp");
            assert_json_roundtrip(&MobileCapabilitiesDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                supported_features: vec![MobileFeatureDto::CorpusSync, MobileFeatureDto::ReviewUpload],
                max_snapshot_page: 500,
                max_review_batch: 500,
                max_nback_batch: 20,
                server_version: "1.0.0".into(),
            });
            assert_json_roundtrip(&DeviceRegistrationRequestDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                device_id: Uuid::from_u128(1),
                display_name: "Phone".into(),
            });
            assert_json_roundtrip(&RegisteredDeviceDto {
                device_id: Uuid::from_u128(1),
                display_name: "Phone".into(),
                created_at: at,
                last_seen_at: at,
                revoked_at: None,
            });
            assert_json_roundtrip(&ProtocolErrorDto {
                supported_protocol_version: MOBILE_PROTOCOL_VERSION,
                kind: ProtocolErrorKindDto::UnsupportedVersion,
                message: "unsupported protocol version".into(),
            });
        }

        #[test]
        fn corpus_pages_json_roundtrip(
            translation_id in 1i32..i32::MAX,
            cursor in 0i64..i64::MAX,
            timestamp in 1_600_000_000i64..2_000_000_000,
        ) {
            let at = DateTime::from_timestamp(timestamp, 0).expect("generated timestamp");
            assert_json_roundtrip(&CorpusSnapshotRequestDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                pair: pair(),
                after_translation_id: 0,
                snapshot_watermark: Some(cursor),
                limit: 500,
            });
            assert_json_roundtrip(&CorpusPageDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                pair: pair(),
                translations: vec![translation(translation_id)],
                next_cursor: translation_id,
                has_more: false,
                snapshot_watermark: cursor,
            });
            assert_json_roundtrip(&CorpusChangePageDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                pair: pair(),
                changes: vec![CorpusChangeDto {
                    sequence: cursor,
                    translation_id,
                    kind: CorpusChangeKindDto::Upsert,
                    translation: Some(translation(translation_id)),
                    changed_at: at,
                }],
                next_cursor: cursor,
                has_more: false,
                change_watermark: cursor,
            });
        }

        #[test]
        fn review_and_card_response_json_roundtrip(
            translation_id in 1i32..i32::MAX,
            cursor in 0i64..i64::MAX,
            timestamp in 1_600_000_000i64..2_000_000_000,
        ) {
            let at = DateTime::from_timestamp(timestamp, 0).expect("generated timestamp");
            assert_json_roundtrip(&ReviewBatchResponseDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                acknowledgements: vec![ReviewEventAckDto {
                    event_id: Uuid::from_u128(2),
                    status: ReviewEventStatusDto::Applied,
                }],
                cards: vec![card(translation_id, at, cursor)],
            });
            assert_json_roundtrip(&CardChangePageDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                changes: vec![CardChangeDto::Upsert {
                    sequence: cursor,
                    card: card(translation_id, at, cursor),
                }],
                next_cursor: cursor,
                has_more: false,
                change_watermark: cursor,
            });
        }

        #[test]
        fn change_requests_json_roundtrip(cursor in 0i64..i64::MAX) {
            assert_json_roundtrip(&CorpusChangeRequestDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                pair: pair(),
                cursor,
                limit: 500,
            });
            assert_json_roundtrip(&CardChangeRequestDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                cursor,
                limit: 500,
            });
        }

        #[test]
        fn nback_batch_json_roundtrip(
            seed in any::<u64>(),
            timestamp in 1_600_000_000i64..2_000_000_000,
        ) {
            let at = DateTime::from_timestamp(timestamp, 0).expect("generated timestamp");
            assert_json_roundtrip(&NbackBatchRequestDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                device_id: Uuid::from_u128(1),
                sessions: vec![NbackSessionUploadDto {
                    client_session_id: Uuid::from_u128(3),
                    pair: pair(),
                    mode: NbackModeDto::AudioWritten,
                    n_level: 2,
                    interval_ms: 4_000,
                    seed,
                    vocabulary_translation_ids: (1..=8).collect(),
                    responses: vec![NbackTrialResponseDto {
                        trial_number: 1,
                        audio_response: Some(false),
                        visual_response: Some(true),
                        response_time_ms: 500,
                    }],
                    started_at: at,
                    completed_at: at,
                }],
            });
            assert_json_roundtrip(&NbackBatchResponseDto {
                protocol_version: MOBILE_PROTOCOL_VERSION,
                acknowledgements: vec![NbackUploadAckDto {
                    client_session_id: Uuid::from_u128(3),
                    status: NbackUploadStatusDto::Applied,
                    result: Some(DnbSessionResultsDto {
                        session_id: 9,
                        mode: crate::DnbModeDto::AudioWritten,
                        n_level_start: 2,
                        n_level_peak: 3,
                        n_level_end: 3,
                        trials_completed: 10,
                        accuracy_audio: Some(0.8),
                        accuracy_visual: Some(0.9),
                        interval_ms_start: 4_000,
                        interval_ms_end: 3_800,
                    }),
                }],
            });
        }

        #[test]
        fn cached_quiz_json_roundtrip(timestamp in 1_600_000_000i64..2_000_000_000) {
            let at = DateTime::from_timestamp(timestamp, 0).expect("generated timestamp");
            assert_json_roundtrip(&CachedQuizDto {
                key: QuizCacheKeyDto {
                    source_sha256: "abc123".into(),
                    native_lang: "en".into(),
                    foreign_lang: "de".into(),
                    kind: QuizKindDto::Cloze,
                    cefr_level: Some("A2".into()),
                    item_count: 1,
                },
                label: "Lesson".into(),
                items: vec![QuizItemDto::Cloze(crate::ClozeQuizDto {
                    sentence_with_blank: "Ich ___ hier".into(),
                    answer: "bin".into(),
                    hint: None,
                    rule_context: None,
                })],
                created_at: at,
            });
        }
    }
}
