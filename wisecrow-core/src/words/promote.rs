//! Provider loop that turns candidates into presentations and links.
//!
//! Generation happens outside any transaction; each result is then committed
//! on its own through [`store::promote_candidate`], so a failure part-way
//! keeps every earlier success and the summary reports what happened.

use std::collections::HashMap;

use sqlx::PgPool;

use super::store::{self, PromotionOutcome, StoredCandidate};
use super::{PromotionOptions, PromotionSummary, Refresh};
use crate::errors::WisecrowError;
use crate::glossing::{PresentationCandidate, PresentationContext, ValidatedPresentation};
use crate::llm::LlmProvider;

/// Candidates sent to the model in one prompt.
const PROMPT_CHUNK: usize = 25;

/// Attempts up to `options.limit` candidates of the pair in evidence order,
/// asking `provider` for presentations with the stored source examples as
/// context, and links each accepted word to a translation row.
///
/// # Errors
///
/// Returns an error when the pair is unsupported, the provider fails a
/// batch, or a database step fails. The summary accumulated so far is logged
/// before the error is returned so the caller never reports success.
pub async fn promote_words(
    pool: &PgPool,
    provider: &dyn LlmProvider,
    native_lang: &str,
    foreign_lang: &str,
    options: &PromotionOptions,
) -> Result<PromotionSummary, WisecrowError> {
    let run = Run {
        pool,
        provider,
        native_lang,
        foreign_lang,
        native_name: language_name(native_lang)?,
        foreign_name: language_name(foreign_lang)?,
        refresh: options.refresh,
    };
    super::extract::resolve_pair(pool, native_lang, foreign_lang).await?;
    let candidates = pending_candidates(pool, native_lang, foreign_lang, options).await?;
    let mut summary = PromotionSummary::default();
    let result = run.promote_all(&candidates, &mut summary).await;
    if let Err(error) = &result {
        tracing::error!(
            %error,
            attempted = summary.attempted,
            accepted = summary.accepted,
            rejected = summary.rejected,
            failed = summary.failed,
            stale = summary.stale,
            "word promotion stopped"
        );
    }
    result.map(|()| summary)
}

fn language_name(code: &str) -> Result<&'static str, WisecrowError> {
    crate::cli::SUPPORTED_LANGUAGE_INFO
        .iter()
        .find(|(candidate, _)| *candidate == code)
        .map(|(_, name)| *name)
        .ok_or_else(|| WisecrowError::UnsupportedLanguage(code.to_owned()))
}

async fn pending_candidates(
    pool: &PgPool,
    native_lang: &str,
    foreign_lang: &str,
    options: &PromotionOptions,
) -> Result<Vec<StoredCandidate>, WisecrowError> {
    let statuses: &[&str] = match options.refresh {
        Refresh::Pending => &["pending"],
        Refresh::RetryFailed => &["pending", "failed"],
        Refresh::All => &["pending", "failed", "accepted", "rejected"],
    };
    // An accepted candidate whose translation was deleted is always eligible
    // for link repair, whatever the mode.
    let rows = sqlx::query_as::<_, StoredCandidate>(
        "SELECT c.id, c.word, c.surface, c.revision, c.occurrence_count,
                c.native_language_id, c.foreign_language_id
         FROM word_candidates c
         JOIN languages n ON n.id = c.native_language_id
         JOIN languages f ON f.id = c.foreign_language_id
         LEFT JOIN word_promotions p ON p.candidate_id = c.id
         WHERE n.code = $1 AND f.code = $2
           AND (c.status = ANY($3::TEXT[])
                OR (c.status = 'accepted' AND p.translation_id IS NULL))
         ORDER BY c.occurrence_count DESC, c.word
         LIMIT $4",
    )
    .bind(native_lang)
    .bind(foreign_lang)
    .bind(statuses)
    .bind(i64::from(options.limit))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Everything one promotion run holds constant across its chunks.
struct Run<'a> {
    pool: &'a PgPool,
    provider: &'a dyn LlmProvider,
    native_lang: &'a str,
    foreign_lang: &'a str,
    native_name: &'a str,
    foreign_name: &'a str,
    refresh: Refresh,
}

impl Run<'_> {
    async fn promote_all(
        &self,
        candidates: &[StoredCandidate],
        summary: &mut PromotionSummary,
    ) -> Result<(), WisecrowError> {
        for chunk in candidates.chunks(PROMPT_CHUNK) {
            let mut ask = Vec::with_capacity(chunk.len());
            for candidate in chunk {
                summary.attempted = summary.attempted.saturating_add(1);
                // Only an explicit refresh replaces a current presentation; a
                // link repair otherwise reuses what is stored.
                let reusable = if self.refresh == Refresh::All {
                    None
                } else {
                    store::current_presentation(
                        self.pool,
                        self.native_lang,
                        self.foreign_lang,
                        &candidate.word,
                    )
                    .await?
                };
                match reusable {
                    Some(presentation) => self.record(candidate, &presentation, summary).await?,
                    None => ask.push(candidate),
                }
            }
            if ask.is_empty() {
                continue;
            }
            let (requested, contexts) = load_requests(self.pool, &ask).await?;
            let generated = crate::glossing::generate_presentations(
                self.provider,
                &requested,
                self.native_name,
                self.foreign_name,
                &contexts,
            )
            .await;
            let generated = match generated {
                Ok(generated) => generated,
                Err(error) => {
                    for candidate in &ask {
                        mark_failed(self.pool, candidate, summary).await?;
                    }
                    return Err(error);
                }
            };
            let mut by_word: HashMap<&str, &ValidatedPresentation> = generated
                .iter()
                .map(|presentation| (presentation.word.as_str(), presentation))
                .collect();
            for candidate in ask {
                match by_word.remove(candidate.word.as_str()) {
                    Some(presentation) => self.record(candidate, presentation, summary).await?,
                    None => mark_failed(self.pool, candidate, summary).await?,
                }
            }
        }
        Ok(())
    }

    /// Commits one result and counts its outcome. A unique-pair conflict on
    /// an owned refresh is the candidate's failure, not the run's.
    async fn record(
        &self,
        candidate: &StoredCandidate,
        presentation: &ValidatedPresentation,
        summary: &mut PromotionSummary,
    ) -> Result<(), WisecrowError> {
        let outcome = store::promote_candidate(
            self.pool,
            candidate,
            presentation,
            self.native_lang,
            self.foreign_lang,
        )
        .await;
        match outcome {
            Ok(PromotionOutcome::Accepted(_)) => {
                summary.accepted = summary.accepted.saturating_add(1);
            }
            Ok(PromotionOutcome::Rejected) => {
                summary.rejected = summary.rejected.saturating_add(1);
            }
            Ok(PromotionOutcome::Stale) => summary.stale = summary.stale.saturating_add(1),
            Err(WisecrowError::Conflict(reason)) => {
                tracing::warn!(word = candidate.word, %reason, "promotion conflict");
                mark_failed(self.pool, candidate, summary).await?;
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }
}

/// Loads the requested words and their stored examples for one prompt.
async fn load_requests(
    pool: &PgPool,
    candidates: &[&StoredCandidate],
) -> Result<(Vec<PresentationCandidate>, Vec<PresentationContext>), WisecrowError> {
    let ids: Vec<i64> = candidates.iter().map(|candidate| candidate.id).collect();
    let examples: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT e.candidate_id, e.native_sentence, e.foreign_sentence
         FROM word_candidate_examples e
         WHERE e.candidate_id = ANY($1)
         ORDER BY e.candidate_id, e.ordinal",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await?;
    let words: HashMap<i64, &str> = candidates
        .iter()
        .map(|candidate| (candidate.id, candidate.word.as_str()))
        .collect();
    let requested = candidates
        .iter()
        .map(|candidate| PresentationCandidate {
            word: candidate.word.clone(), // clone: prompt input owned per batch
            surface: candidate.surface.clone(), // clone: prompt input owned per batch
        })
        .collect();
    let contexts = examples
        .into_iter()
        .filter_map(|(candidate_id, native_sentence, foreign_sentence)| {
            words.get(&candidate_id).map(|word| PresentationContext {
                word: (*word).to_owned(),
                native_sentence,
                foreign_sentence,
            })
        })
        .collect();
    Ok((requested, contexts))
}

/// Marks a candidate failed only while it is still the revision that was
/// asked about; a re-extracted candidate is stale instead.
async fn mark_failed(
    pool: &PgPool,
    candidate: &StoredCandidate,
    summary: &mut PromotionSummary,
) -> Result<(), WisecrowError> {
    let updated =
        sqlx::query("UPDATE word_candidates SET status = 'failed' WHERE id = $1 AND revision = $2")
            .bind(candidate.id)
            .bind(candidate.revision)
            .execute(pool)
            .await?
            .rows_affected();
    if updated == 1 {
        summary.failed = summary.failed.saturating_add(1);
    } else {
        summary.stale = summary.stale.saturating_add(1);
    }
    Ok(())
}
