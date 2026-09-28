//! Speech for a language's grammar example sentences.
//!
//! A point's correct examples are read out to the learner; its incorrect
//! ones never are, so a wrong form is not heard spoken as though it were
//! right. Clips are keyed by the sentence's audio fingerprint, so a sentence
//! two points quote is spoken once and a point re-imported with the same
//! sentences finds its clips where it left them. Reclaiming clips no current
//! example needs is a deliberate `prune`, never a side effect of a run.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use tokio::sync::watch;
use tracing::info;

use crate::errors::WisecrowError;
use crate::grammar::rules::{CorrectExample, RuleRepository};
use crate::media::cache::MediaCache;
use crate::media::fingerprint::MediaFingerprint;
use crate::media::prefetch::{progress_bar, run_batch, PrefetchMode};
use crate::media::MediaType;

/// Turns a sentence into speech, and says which clip that would be.
///
/// The fingerprint and the bytes come from the same profile, so a speaker
/// that would voice a sentence differently names a different clip.
#[async_trait]
pub trait SentenceSpeaker: Send + Sync {
    /// The clip identity for `sentence` in `language` under this speaker.
    ///
    /// # Errors
    ///
    /// Returns an error when the language has no voice.
    fn fingerprint(
        &self,
        language: &str,
        sentence: &str,
    ) -> Result<MediaFingerprint, WisecrowError>;

    /// The clip bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when synthesis fails.
    async fn speak(&self, language: &str, sentence: &str) -> Result<Vec<u8>, WisecrowError>;
}

/// The deployment's speech profile: CereProc where configured, Edge
/// elsewhere, exactly as the vocabulary path chooses.
#[cfg(feature = "tts")]
pub struct TtsSpeaker {
    cereproc: Option<crate::media::cereproc::CereprocClient>,
}

#[cfg(feature = "tts")]
impl TtsSpeaker {
    #[must_use]
    pub fn new(cereproc: Option<crate::media::cereproc::CereprocClient>) -> Self {
        Self { cereproc }
    }

    fn subject(language: &str, sentence: &str) -> crate::media::MediaSubject {
        crate::media::MediaSubject {
            to_phrase: sentence.to_owned(),
            from_phrase: String::new(),
            foreign_lang: language.to_owned(),
            image_query: None,
            is_phrase: true,
        }
    }
}

#[cfg(feature = "tts")]
#[async_trait]
impl SentenceSpeaker for TtsSpeaker {
    fn fingerprint(
        &self,
        language: &str,
        sentence: &str,
    ) -> Result<MediaFingerprint, WisecrowError> {
        crate::media::audio_cache_key(&Self::subject(language, sentence), self.cereproc.as_ref())
    }

    async fn speak(&self, language: &str, sentence: &str) -> Result<Vec<u8>, WisecrowError> {
        crate::media::audio::generate_tts(sentence, language, self.cereproc.as_ref()).await
    }
}

/// What one run is asked to do.
#[derive(Debug, Clone)]
pub struct GrammarAudioOptions {
    pub language: String,
    pub level: Option<String>,
    pub mode: PrefetchMode,
    /// Reclaim clips no current correct example, of any language, needs.
    pub prune: bool,
}

/// What became of one distinct sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentenceOutcome {
    Cached,
    Missing,
    Generated,
    Unsupported,
    Failed,
}

/// One distinct sentence and how it fared.
#[derive(Debug, Clone)]
pub struct SentenceReport {
    pub sentence: String,
    pub outcome: SentenceOutcome,
}

/// The run's tally.
#[derive(Debug, Default)]
pub struct GrammarAudioSummary {
    /// Distinct sentences selected.
    pub selected: usize,
    /// Example rows those sentences came from.
    pub examples: usize,
    pub reports: Vec<SentenceReport>,
    pub generated_bytes: u64,
    pub pruned: usize,
}

impl GrammarAudioSummary {
    #[must_use]
    pub fn count(&self, outcome: SentenceOutcome) -> usize {
        self.reports
            .iter()
            .filter(|report| report.outcome == outcome)
            .count()
    }

    /// Whether anything failed or could not be voiced.
    #[must_use]
    pub fn needs_attention(&self) -> bool {
        self.reports.iter().any(|report| {
            matches!(
                report.outcome,
                SentenceOutcome::Failed | SentenceOutcome::Unsupported
            )
        })
    }
}

/// The correct examples of `language`, optionally of one level.
///
/// # Errors
///
/// Returns an error when the query fails.
pub async fn correct_examples(
    pool: &PgPool,
    language: &str,
    level: Option<&str>,
) -> Result<Vec<CorrectExample>, WisecrowError> {
    RuleRepository::correct_examples(pool, language, level).await
}

/// One distinct sentence to voice.
struct Job {
    fingerprint: MediaFingerprint,
    language: String,
    sentence: String,
}

/// Voices every correct example of the language once, four at a time, and
/// prunes when asked.
///
/// The cache and the speaker are shared through `Arc` because the runner
/// spawns each sentence onto the runtime, and a spawned future owns what it
/// touches.
///
/// # Errors
///
/// Returns an error when the language code is invalid, the selection fails,
/// a worker cannot be joined, the run is cancelled, or a prune fails.
pub async fn prefetch_grammar_audio(
    pool: &PgPool,
    cache: Arc<MediaCache>,
    speaker: Arc<dyn SentenceSpeaker>,
    options: &GrammarAudioOptions,
    cancel: watch::Receiver<bool>,
) -> Result<GrammarAudioSummary, WisecrowError> {
    if !crate::lang::is_valid_code(&options.language) {
        return Err(WisecrowError::InvalidInput(format!(
            "Invalid language code: {}",
            options.language
        )));
    }
    let examples = correct_examples(pool, &options.language, options.level.as_deref()).await?;
    let (jobs, unsupported) = distinct_jobs(speaker.as_ref(), &examples);
    info!(
        "Voicing {} distinct sentences from {} {} examples",
        jobs.len(),
        examples.len(),
        options.language
    );

    let mut summary = GrammarAudioSummary {
        selected: jobs.len().saturating_add(unsupported.len()),
        examples: examples.len(),
        ..GrammarAudioSummary::default()
    };
    for sentence in unsupported {
        summary.reports.push(SentenceReport {
            sentence,
            outcome: SentenceOutcome::Unsupported,
        });
    }

    let progress = progress_bar(jobs.len())?;
    let mode = options.mode;
    let outcomes = run_batch(
        jobs,
        |job| {
            let cache = Arc::clone(&cache); // clone: each worker shares the one cache
            let speaker = Arc::clone(&speaker); // clone: each worker shares the one speaker
            let progress = progress.clone(); // clone: ProgressBar is Arc-based
            async move {
                let report = voice(&cache, speaker.as_ref(), mode, job).await;
                progress.inc(1);
                report
            }
        },
        cancel,
    )
    .await?;
    for (report, bytes) in outcomes {
        summary.generated_bytes = summary.generated_bytes.saturating_add(bytes);
        summary.reports.push(report);
    }
    progress.finish_with_message("done");

    if options.prune && mode == PrefetchMode::Execute {
        summary.pruned = prune(pool, &cache, speaker.as_ref()).await?;
    }
    Ok(summary)
}

/// One job per distinct fingerprint, and the sentences no voice covers.
fn distinct_jobs(
    speaker: &dyn SentenceSpeaker,
    examples: &[CorrectExample],
) -> (Vec<Job>, Vec<String>) {
    let mut seen: HashSet<String> = HashSet::new();
    let mut jobs = Vec::new();
    let mut unsupported = Vec::new();
    for example in examples {
        match speaker.fingerprint(&example.language, &example.sentence) {
            Ok(fingerprint) => {
                if seen.insert(fingerprint.as_str().to_owned()) {
                    jobs.push(Job {
                        fingerprint,
                        language: example.language.clone(), // clone: the job owns its inputs
                        sentence: example.sentence.clone(), // clone: the job owns its inputs
                    });
                }
            }
            Err(_) => {
                if !unsupported.contains(&example.sentence) {
                    unsupported.push(example.sentence.clone()); // clone: reported by sentence
                }
            }
        }
    }
    (jobs, unsupported)
}

/// Probe or generate one sentence; returns the report and the bytes generated.
async fn voice(
    cache: &MediaCache,
    speaker: &dyn SentenceSpeaker,
    mode: PrefetchMode,
    job: Job,
) -> (SentenceReport, u64) {
    let Job {
        fingerprint,
        language,
        sentence,
    } = job;
    if mode == PrefetchMode::Preview {
        let outcome = match cache.probe_sentence(&fingerprint, MediaType::Audio).await {
            Ok(true) => SentenceOutcome::Cached,
            Ok(false) => SentenceOutcome::Missing,
            Err(error) => {
                tracing::warn!(error = %error, "Sentence cache probe failed");
                SentenceOutcome::Failed
            }
        };
        return (SentenceReport { sentence, outcome }, 0);
    }
    let mut generated = 0u64;
    let result = cache
        .get_or_fetch_sentence(
            &fingerprint,
            MediaType::Audio,
            &language,
            &sentence,
            || async {
                let bytes = speaker.speak(&language, &sentence).await?;
                generated = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                Ok(bytes)
            },
        )
        .await;
    let outcome = match result {
        Ok(_) if generated > 0 => SentenceOutcome::Generated,
        Ok(_) => SentenceOutcome::Cached,
        Err(error) => {
            tracing::warn!(error = %error, "Sentence speech failed");
            SentenceOutcome::Failed
        }
    };
    (SentenceReport { sentence, outcome }, generated)
}

/// Removes clips no current correct example of any language names.
async fn prune(
    pool: &PgPool,
    cache: &MediaCache,
    speaker: &dyn SentenceSpeaker,
) -> Result<usize, WisecrowError> {
    let languages: Vec<String> = sqlx::query_scalar("SELECT code FROM languages ORDER BY code")
        .fetch_all(pool)
        .await?;
    let mut keep = Vec::new();
    for language in languages {
        let examples = correct_examples(pool, &language, None).await?;
        let (jobs, _) = distinct_jobs(speaker, &examples);
        keep.extend(jobs.into_iter().map(|job| job.fingerprint));
    }
    cache.prune_sentences(&keep).await
}
