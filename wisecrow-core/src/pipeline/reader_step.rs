//! One wake of `prepare-readers`.
//!
//! The order is fixed: store one available book, else translate one run,
//! else gloss one batch, else voice one sentence batch, else voice one word
//! batch. The shelf is English. Each pair's target language is what that
//! English is translated into. A PDF on the shelf is
//! available when its level maps to CEFR and its `Translate` cell is not `no`.

use std::path::Path;

use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::llm::cache::CachedLlm;
use crate::media::grammar_audio::SentenceSpeaker;
use crate::pipeline::lock::with_pair_lock;
use crate::pipeline::prepare::CorpusStep;
use crate::pipeline::PairSpec;
use crate::reader::audio::{voice_sentence_batch, voice_word_batch, SENTENCE_BATCH, WORD_BATCH};
use crate::reader::extract::{extract, Extraction};
use crate::reader::import::{
    book_exists, store_book, translate_book_run, BookStatus, ImportIdentity, NewBook, RunStored,
    AUTO_PAGES,
};
use crate::reader::segment::{segment, SegmentedSentence, SEGMENTER_VERSION};
use crate::reader::shelf::{shelf_entries, ReaderEntry};

/// Does one piece of reader work and returns what it was.
///
/// `LlmUnavailable` is returned as itself. Extraction takes no pair lock;
/// a translate, gloss or voice write does.
///
/// # Errors
///
/// Returns an error when the ledger, a book file, the model or the database
/// fails. A usage limit is [`WisecrowError::LlmUnavailable`].
pub async fn prepare_reader_step(
    pool: &PgPool,
    shelf: &Path,
    pairs: &[PairSpec],
    model: &CachedLlm<'_>,
    speaker: Option<&dyn SentenceSpeaker>,
) -> Result<CorpusStep, WisecrowError> {
    if extract_one(pool, shelf).await? {
        return Ok(CorpusStep::Worked);
    }
    if translate_one(pool, pairs, model).await? {
        return Ok(CorpusStep::Worked);
    }
    if gloss_one(pool, pairs, model).await? {
        return Ok(CorpusStep::Worked);
    }
    if let Some(speaker) = speaker {
        if voice_one(pool, pairs, speaker).await? {
            return Ok(CorpusStep::Worked);
        }
    }
    Ok(CorpusStep::Idle)
}

async fn extract_one(pool: &PgPool, shelf: &Path) -> Result<bool, WisecrowError> {
    for entry in shelf_entries(shelf)? {
        let Some(&cefr) = entry.cefr.as_ref().ok() else {
            continue;
        };
        let pages_label = match &entry.pages {
            Ok(Some(range)) => range.label(),
            Ok(None) => AUTO_PAGES.to_owned(),
            Err(_) => continue,
        };
        let bytes = std::fs::read(entry.path(shelf))?;
        let identity = ImportIdentity {
            file_sha256: Sha256::digest(&bytes).into(),
            pages: &pages_label,
            segmenter_version: SEGMENTER_VERSION,
        };
        if book_exists(pool, &identity.key()).await? {
            continue;
        }
        let document = crate::grammar::pdf_text::layout_pages(&entry.path(shelf))?;
        let range = match &entry.pages {
            Ok(pages) => *pages,
            Err(_) => continue,
        };
        let worked = match extract(&document, range) {
            Extraction::Thin { .. } => {
                store_revision(pool, &entry, identity, cefr, BookStatus::Thin, None, &[]).await?
            }
            Extraction::NeedsPages => {
                store_revision(
                    pool,
                    &entry,
                    identity,
                    cefr,
                    BookStatus::NeedsPages,
                    None,
                    &[],
                )
                .await?
            }
            Extraction::Body(body) => {
                let sentences = segment(&body);
                store_revision(
                    pool,
                    &entry,
                    identity,
                    cefr,
                    BookStatus::Ready,
                    Some((body.first_page, body.last_page)),
                    &sentences,
                )
                .await?
            }
        };
        if worked {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn store_revision(
    pool: &PgPool,
    entry: &ReaderEntry,
    identity: ImportIdentity<'_>,
    cefr: &'static str,
    status: BookStatus,
    pages: Option<(u32, u32)>,
    sentences: &[SegmentedSentence],
) -> Result<bool, WisecrowError> {
    Ok(store_book(
        pool,
        &NewBook {
            entry,
            identity,
            cefr_level: cefr,
            status,
            pages,
            sentences,
        },
    )
    .await?
    .is_some())
}

async fn translate_one(
    pool: &PgPool,
    pairs: &[PairSpec],
    model: &CachedLlm<'_>,
) -> Result<bool, WisecrowError> {
    for pair in pairs {
        let books: Vec<(i32, String)> = sqlx::query_as(
            "SELECT b.id, c.code
             FROM books b
             JOIN cefr_levels c ON c.id = b.cefr_level_id
             WHERE b.status = 'ready'
               AND (
                   SELECT COUNT(*) FROM book_sentences s WHERE s.book_id = b.id
               ) > COALESCE((
                   SELECT p.cursor
                   FROM book_import_progress p
                   JOIN languages l ON l.id = p.language_id
                   WHERE p.book_id = b.id AND l.code = $1
               ), 0)
             ORDER BY b.id",
        )
        .bind(&pair.foreign)
        .fetch_all(pool)
        .await?;
        let language_name = language_name(&pair.foreign)?;
        for (book_id, cefr) in &books {
            let stored = with_pair_lock(pool, &pair.native, &pair.foreign, || {
                translate_book_run(pool, *book_id, &pair.foreign, language_name, cefr, model)
            })
            .await?;
            match stored {
                RunStored::Done => {}
                RunStored::Advanced { .. } | RunStored::Rejected { .. } => return Ok(true),
            }
        }
    }
    Ok(false)
}

async fn gloss_one(
    pool: &PgPool,
    pairs: &[PairSpec],
    model: &CachedLlm<'_>,
) -> Result<bool, WisecrowError> {
    for pair in pairs {
        let stored = with_pair_lock(pool, &pair.native, &pair.foreign, || {
            crate::reader::notes::gloss_forms(pool, &pair.foreign, &pair.native, model)
        })
        .await?;
        if matches!(stored, crate::reader::notes::GlossStored::Stored { .. }) {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn voice_one(
    pool: &PgPool,
    pairs: &[PairSpec],
    speaker: &dyn SentenceSpeaker,
) -> Result<bool, WisecrowError> {
    let cache_dir = crate::media::cache::default_cache_dir();
    for pair in pairs {
        let voiced = with_pair_lock(pool, &pair.native, &pair.foreign, || {
            voice_sentence_batch(pool, &pair.foreign, speaker, &cache_dir, SENTENCE_BATCH)
        })
        .await?;
        if voiced > 0 {
            return Ok(true);
        }
    }
    for pair in pairs {
        let voiced = with_pair_lock(pool, &pair.native, &pair.foreign, || {
            voice_word_batch(pool, &pair.foreign, speaker, &cache_dir, WORD_BATCH)
        })
        .await?;
        if voiced > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether one `prepare-readers` wake would find work, without calling a model.
///
/// The order matches [`prepare_reader_step`]: an unstored available book, a
/// ready book whose cursor is behind its sentences, a missing gloss, then a
/// null audio fingerprint when `voice` is set. A failed run by itself is not
/// work.
///
/// # Errors
///
/// Returns an error when the ledger or the database cannot be read.
pub async fn reader_has_work(
    pool: &PgPool,
    shelf: &Path,
    pairs: &[PairSpec],
    voice: bool,
) -> Result<bool, WisecrowError> {
    for entry in shelf_entries(shelf)? {
        let pages_label = match &entry.pages {
            Ok(Some(range)) => range.label(),
            Ok(None) => AUTO_PAGES.to_owned(),
            Err(_) => continue,
        };
        let bytes = std::fs::read(entry.path(shelf))?;
        let identity = ImportIdentity {
            file_sha256: Sha256::digest(&bytes).into(),
            pages: &pages_label,
            segmenter_version: SEGMENTER_VERSION,
        };
        if !book_exists(pool, &identity.key()).await? {
            return Ok(true);
        }
    }
    for pair in pairs {
        if translation_pending(pool, &pair.foreign).await? || gloss_pending(pool, pair).await? {
            return Ok(true);
        }
        if voice && audio_pending(pool, &pair.foreign).await? {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn translation_pending(pool: &PgPool, foreign: &str) -> Result<bool, WisecrowError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
             FROM books b
             WHERE b.status = 'ready'
               AND (
                   SELECT COUNT(*) FROM book_sentences s WHERE s.book_id = b.id
               ) > COALESCE((
                   SELECT p.cursor
                   FROM book_import_progress p
                   JOIN languages l ON l.id = p.language_id
                   WHERE p.book_id = b.id AND l.code = $1
               ), 0)
         )",
    )
    .bind(foreign)
    .fetch_one(pool)
    .await?)
}

async fn gloss_pending(pool: &PgPool, pair: &PairSpec) -> Result<bool, WisecrowError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
             FROM book_tokens k
             JOIN book_translations t ON t.id = k.translation_id
             JOIN languages fl ON fl.id = t.language_id
             JOIN languages nl ON nl.code = $2
             WHERE fl.code = $1
               AND NOT EXISTS (
                   SELECT 1 FROM book_glosses g
                   WHERE g.language_id = fl.id
                     AND g.native_language_id = nl.id
                     AND g.normalised = k.normalised
               )
         )",
    )
    .bind(&pair.foreign)
    .bind(&pair.native)
    .fetch_one(pool)
    .await?)
}

async fn audio_pending(pool: &PgPool, foreign: &str) -> Result<bool, WisecrowError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
             FROM book_translations bt
             JOIN languages l ON l.id = bt.language_id
             WHERE l.code = $1 AND bt.audio_fingerprint IS NULL
         ) OR EXISTS (
             SELECT 1
             FROM book_glosses bg
             JOIN languages l ON l.id = bg.language_id
             WHERE l.code = $1 AND bg.status = 'ok' AND bg.audio_fingerprint IS NULL
         )",
    )
    .bind(foreign)
    .fetch_one(pool)
    .await?)
}

fn language_name(code: &str) -> Result<&'static str, WisecrowError> {
    crate::cli::SUPPORTED_LANGUAGE_INFO
        .iter()
        .find(|(candidate, _)| *candidate == code)
        .map(|(_, name)| *name)
        .ok_or_else(|| WisecrowError::UnsupportedLanguage(code.to_owned()))
}
