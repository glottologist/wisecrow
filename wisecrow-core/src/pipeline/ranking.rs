//! Checksum of the corpus-evidence rows a ranking run reads.
//!
//! The digest is streamed in primary-key order. A replacement that keeps the
//! row count still changes it, and a language the size of French is never
//! concatenated into one string.

use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::frequency::FrequencyUpdater;
use crate::lang;

use super::lock::with_pair_lock;

const PAGE_LEN: i64 = 5_000;
const DOMAIN: &[u8] = b"wisecrow-pipeline-evidence-v1";

struct EvidenceDigest {
    checksum: [u8; 32],
    rows: u64,
}

/// SHA-256 of the evidence rows whose native or foreign side is `foreign`.
///
/// # Errors
///
/// Returns an error when `foreign` is not a well-formed language code or a
/// query fails.
pub async fn evidence_checksum(pool: &PgPool, foreign: &str) -> Result<[u8; 32], WisecrowError> {
    Ok(evidence_digest(pool, foreign).await?.checksum)
}

async fn evidence_digest(pool: &PgPool, foreign: &str) -> Result<EvidenceDigest, WisecrowError> {
    if !lang::is_valid_code(foreign) {
        return Err(WisecrowError::InvalidInput(format!(
            "Invalid language code: {foreign}"
        )));
    }

    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    let mut after_id = 0i32;
    let mut rows = 0u64;

    loop {
        let page: Vec<(i32, String, String)> = sqlx::query_as(
            "SELECT t.id, t.from_phrase, t.to_phrase
               FROM corpus_evidence_translations t
               JOIN languages fl ON fl.id = t.from_language_id
               JOIN languages tl ON tl.id = t.to_language_id
              WHERE (fl.code = $1 OR tl.code = $1) AND t.id > $2
              ORDER BY t.id
              LIMIT $3",
        )
        .bind(foreign)
        .bind(after_id)
        .bind(PAGE_LEN)
        .fetch_all(pool)
        .await?;

        let Some((last_id, _, _)) = page.last() else {
            break;
        };
        after_id = *last_id;

        for (id, from_phrase, to_phrase) in &page {
            rows = rows.checked_add(1).ok_or_else(|| {
                WisecrowError::InvalidInput("evidence row count overflow".to_owned())
            })?;
            hasher.update(id.to_be_bytes());
            hasher.update([0u8]);
            hasher.update(from_phrase.as_bytes());
            hasher.update([0u8]);
            hasher.update(to_phrase.as_bytes());
            hasher.update([0u8]);
        }

        if page.len() < usize::try_from(PAGE_LEN).unwrap_or(usize::MAX) {
            break;
        }
    }

    hasher.update(rows.to_be_bytes());
    let mut checksum = [0u8; 32];
    checksum.copy_from_slice(&hasher.finalize());
    Ok(EvidenceDigest { checksum, rows })
}

async fn stamp_matches(
    pool: &PgPool,
    foreign: &str,
    checksum: &[u8; 32],
) -> Result<bool, WisecrowError> {
    let stored: Option<Vec<u8>> = sqlx::query_scalar(
        "SELECT evidence_checksum FROM pipeline_rank_stamps WHERE foreign_code = $1",
    )
    .bind(foreign)
    .fetch_optional(pool)
    .await?;
    Ok(stored.as_deref() == Some(checksum.as_slice()))
}

pub(crate) async fn rank_is_current(pool: &PgPool, foreign: &str) -> Result<bool, WisecrowError> {
    let checksum = evidence_checksum(pool, foreign).await?;
    stamp_matches(pool, foreign, &checksum).await
}

async fn store_stamp(
    pool: &PgPool,
    foreign: &str,
    checksum: &[u8; 32],
) -> Result<(), WisecrowError> {
    sqlx::query(
        "INSERT INTO pipeline_rank_stamps (foreign_code, evidence_checksum)
         VALUES ($1, $2)
         ON CONFLICT (foreign_code) DO UPDATE
            SET evidence_checksum = EXCLUDED.evidence_checksum,
                ranked_at = CURRENT_TIMESTAMP",
    )
    .bind(foreign)
    .bind(checksum.as_slice())
    .execute(pool)
    .await?;
    Ok(())
}

/// Ranks `foreign` when the stored stamp does not match the evidence digest.
///
/// An empty corpus stores the digest and does not call `frequency --from-corpus`,
/// which rejects a language with nothing to count. Returns `true` when this
/// call ranked or stored a new stamp.
///
/// # Errors
///
/// Returns an error when the checksum, the frequency update, or the stamp
/// write fails.
pub async fn rank_if_stale(
    pool: &PgPool,
    native: &str,
    foreign: &str,
) -> Result<bool, WisecrowError> {
    let digest = evidence_digest(pool, foreign).await?;
    if stamp_matches(pool, foreign, &digest.checksum).await? {
        return Ok(false);
    }

    with_pair_lock(pool, native, foreign, || async {
        let digest = evidence_digest(pool, foreign).await?;
        if stamp_matches(pool, foreign, &digest.checksum).await? {
            return Ok(false);
        }
        if digest.rows > 0 {
            FrequencyUpdater::update_from_corpus(pool, foreign).await?;
        }
        store_stamp(pool, foreign, &digest.checksum).await?;
        Ok(true)
    })
    .await
}
