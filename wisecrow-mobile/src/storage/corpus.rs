use async_trait::async_trait;
use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::{FromRow, Sqlite, SqlitePool, Transaction};
use wisecrow_dto::{
    CardChangeDto, CardChangePageDto, CardSnapshotDto, CardStatusDto, CorpusChangeKindDto,
    CorpusChangePageDto, CorpusPageDto, CorpusTranslationDto, LanguageInfo, LanguagePairDto,
    ScriptDirection, MOBILE_PROTOCOL_VERSION,
};

use super::{
    models::{CorpusEstimate, PairStatus, PairSyncState, SyncErrorKind, SyncPhase},
    sqlite::{active_scope, active_scope_from_pool, interleave_ranked, StoreScope as Scope},
    SqliteStore,
};
use crate::application::{CorpusRepository, MobileError};

const MAX_PAGE_ITEMS: usize = 500;
const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_QUERY_LIMIT: u16 = 500;

#[derive(FromRow)]
struct PairCheckpoint {
    status: String,
    snapshot_watermark: Option<i64>,
    snapshot_after_id: i32,
    change_cursor: i64,
}

#[derive(FromRow)]
struct TranslationRow {
    translation_id: i32,
    from_phrase: String,
    to_phrase: String,
    frequency: i32,
    is_phrase: bool,
}

impl TranslationRow {
    fn into_dto(self) -> CorpusTranslationDto {
        CorpusTranslationDto {
            translation_id: self.translation_id,
            from_phrase: self.from_phrase,
            to_phrase: self.to_phrase,
            frequency: self.frequency,
            is_phrase: self.is_phrase,
        }
    }
}

#[derive(FromRow)]
struct EstimateRow {
    translation_count: i64,
    text_bytes: i64,
}

#[derive(FromRow)]
struct PairSyncRow {
    native_lang: String,
    foreign_lang: String,
    status: String,
    snapshot_watermark: Option<i64>,
    snapshot_after_id: i32,
    change_cursor: i64,
}

impl PairSyncRow {
    fn into_state(self) -> Result<PairSyncState, MobileError> {
        Ok(PairSyncState {
            pair: LanguagePairDto {
                native_lang: self.native_lang,
                foreign_lang: self.foreign_lang,
            },
            status: parse_status(&self.status)?,
            snapshot_watermark: self.snapshot_watermark,
            snapshot_after_id: self.snapshot_after_id,
            change_cursor: self.change_cursor,
        })
    }
}

#[async_trait]
impl CorpusRepository for SqliteStore {
    async fn save_languages(&self, languages: &[LanguageInfo]) -> Result<(), MobileError> {
        validate_languages(languages)?;
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        sqlx::query("DELETE FROM languages WHERE profile_id = ? AND user_id = ?")
            .bind(scope.profile_id)
            .bind(scope.user_id)
            .execute(&mut *transaction)
            .await?;
        for language in languages {
            insert_language(&mut transaction, scope, language).await?;
        }
        transaction.commit().await?;
        Ok(())
    }

    async fn begin_snapshot(
        &self,
        pair: &LanguagePairDto,
        snapshot_watermark: i64,
        estimated_bytes: Option<u64>,
    ) -> Result<(), MobileError> {
        validate_pair(pair)?;
        if snapshot_watermark < 0 {
            return Err(invalid_page());
        }
        let estimated_bytes = optional_i64(estimated_bytes)?;
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        begin_snapshot(
            &mut transaction,
            scope,
            pair,
            snapshot_watermark,
            estimated_bytes,
        )
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn apply_snapshot_page(&self, page: &CorpusPageDto) -> Result<(), MobileError> {
        validate_snapshot_envelope(page)?;
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        apply_snapshot(&mut transaction, scope, page).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn apply_change_page(&self, page: &CorpusChangePageDto) -> Result<(), MobileError> {
        validate_change_envelope(page)?;
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        apply_changes(&mut transaction, scope, page).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn apply_card_page(&self, page: &CardChangePageDto) -> Result<(), MobileError> {
        validate_card_envelope(page)?;
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        apply_cards(&mut transaction, scope, page).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn pair_status(&self, pair: &LanguagePairDto) -> Result<PairStatus, MobileError> {
        validate_pair(pair)?;
        let scope = active_scope_from_pool(&self.pool).await?;
        let status = sqlx::query_scalar::<_, String>(
            "SELECT status FROM language_pairs
             WHERE profile_id = ? AND user_id = ? AND native = ? AND \"foreign\" = ?",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(&pair.native_lang)
        .bind(&pair.foreign_lang)
        .fetch_optional(&self.pool)
        .await?;
        status.map_or(Ok(PairStatus::Absent), |value| parse_status(&value))
    }

    async fn corpus_estimate(&self, pair: &LanguagePairDto) -> Result<CorpusEstimate, MobileError> {
        validate_pair(pair)?;
        let scope = active_scope_from_pool(&self.pool).await?;
        let row = estimate_row(&self.pool, scope, pair).await?;
        Ok(CorpusEstimate {
            translation_count: u64::try_from(row.translation_count).map_err(|_| invalid_state())?,
            text_bytes: u64::try_from(row.text_bytes).map_err(|_| invalid_state())?,
        })
    }

    async fn ranked_translations(
        &self,
        pair: &LanguagePairDto,
        limit: u16,
    ) -> Result<Vec<CorpusTranslationDto>, MobileError> {
        validate_pair(pair)?;
        validate_limit(limit)?;
        let scope = active_scope_from_pool(&self.pool).await?;
        let rows = ranked_rows(&self.pool, scope, pair, limit).await?;
        Ok(rows.into_iter().map(TranslationRow::into_dto).collect())
    }

    async fn translation(
        &self,
        pair: &LanguagePairDto,
        translation_id: i32,
    ) -> Result<Option<CorpusTranslationDto>, MobileError> {
        validate_pair(pair)?;
        if translation_id <= 0 {
            return Err(invalid_page());
        }
        let scope = active_scope_from_pool(&self.pool).await?;
        Ok(translation_row(&self.pool, scope, pair, translation_id)
            .await?
            .map(TranslationRow::into_dto))
    }

    async fn sync_pairs(&self) -> Result<Vec<PairSyncState>, MobileError> {
        let scope = active_scope_from_pool(&self.pool).await?;
        let rows = sqlx::query_as::<_, PairSyncRow>(
            "SELECT native AS native_lang, \"foreign\" AS foreign_lang, status,
                    snapshot_watermark, snapshot_after_id, change_cursor
             FROM language_pairs WHERE profile_id = ? AND user_id = ?
             ORDER BY native, \"foreign\"",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(PairSyncRow::into_state).collect()
    }

    async fn card_cursor(&self) -> Result<i64, MobileError> {
        let scope = active_scope_from_pool(&self.pool).await?;
        let cursor = sqlx::query_scalar(
            "SELECT card_cursor FROM sync_state WHERE profile_id = ? AND user_id = ?",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(cursor.unwrap_or_default())
    }

    async fn sync_phase(&self) -> Result<SyncPhase, MobileError> {
        let scope = active_scope_from_pool(&self.pool).await?;
        let phase = sqlx::query_scalar::<_, String>(
            "SELECT phase FROM sync_state WHERE profile_id = ? AND user_id = ?",
        )
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .fetch_optional(&self.pool)
        .await?;
        phase
            .as_deref()
            .map_or(Ok(SyncPhase::Idle), parse_sync_phase)
    }

    async fn advance_sync_phase(
        &self,
        expected: SyncPhase,
        next: SyncPhase,
    ) -> Result<(), MobileError> {
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        ensure_sync_state(&mut transaction, scope).await?;
        let result = sqlx::query(
            "UPDATE sync_state SET phase = ?
             WHERE profile_id = ? AND user_id = ? AND phase = ?",
        )
        .bind(sync_phase_value(next))
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(sync_phase_value(expected))
        .execute(&mut *transaction)
        .await?;
        require_one_row(result.rows_affected())?;
        transaction.commit().await?;
        Ok(())
    }

    async fn record_sync_success(&self, completed_at: DateTime<Utc>) -> Result<(), MobileError> {
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        ensure_sync_state(&mut transaction, scope).await?;
        sqlx::query(
            "UPDATE sync_state SET phase = 'idle', last_success_at = ?,
                    last_error_kind = NULL, last_error_at = NULL
             WHERE profile_id = ? AND user_id = ?",
        )
        .bind(completed_at)
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn record_sync_error(
        &self,
        kind: SyncErrorKind,
        failed_at: DateTime<Utc>,
    ) -> Result<(), MobileError> {
        let mut transaction = self.pool.begin().await?;
        let scope = active_scope(&mut transaction).await?;
        ensure_sync_state(&mut transaction, scope).await?;
        sqlx::query(
            "UPDATE sync_state SET last_error_kind = ?, last_error_at = ?
             WHERE profile_id = ? AND user_id = ?",
        )
        .bind(sync_error_value(kind))
        .bind(failed_at)
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }
}

async fn insert_language(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
    language: &LanguageInfo,
) -> Result<(), MobileError> {
    sqlx::query(
        "INSERT INTO languages
             (profile_id, user_id, code, name, script_direction) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&language.code)
    .bind(&language.name)
    .bind(script_direction_value(language.script_direction))
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn begin_snapshot(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
    pair: &LanguagePairDto,
    watermark: i64,
    estimated_bytes: Option<i64>,
) -> Result<(), MobileError> {
    sqlx::query(
        "DELETE FROM translations
         WHERE profile_id = ? AND user_id = ? AND native = ? AND \"foreign\" = ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&pair.native_lang)
    .bind(&pair.foreign_lang)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        "INSERT INTO language_pairs
             (profile_id, user_id, native, \"foreign\", status, snapshot_watermark,
              snapshot_after_id, change_cursor, estimated_bytes, updated_at)
         VALUES (?, ?, ?, ?, 'downloading', ?, 0, 0, ?, ?)
         ON CONFLICT(profile_id, user_id, native, \"foreign\") DO UPDATE SET
             status = 'downloading', snapshot_watermark = excluded.snapshot_watermark,
             snapshot_after_id = 0, change_cursor = 0,
             estimated_bytes = excluded.estimated_bytes, updated_at = excluded.updated_at",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&pair.native_lang)
    .bind(&pair.foreign_lang)
    .bind(watermark)
    .bind(estimated_bytes)
    .bind(Utc::now())
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn apply_snapshot(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
    page: &CorpusPageDto,
) -> Result<(), MobileError> {
    let checkpoint = pair_checkpoint(transaction, scope, &page.pair).await?;
    if checkpoint.snapshot_watermark != Some(page.snapshot_watermark) {
        return Err(conflict());
    }
    let status = parse_status(&checkpoint.status)?;
    if page.next_cursor == checkpoint.snapshot_after_id {
        if page.has_more && page.translations.is_empty() {
            return Err(conflict());
        }
        if !page.has_more && status == PairStatus::Downloading {
            return advance_snapshot(transaction, scope, page, checkpoint.snapshot_after_id).await;
        }
        return Ok(());
    }
    if status != PairStatus::Downloading {
        return Err(conflict());
    }
    validate_snapshot_progress(page, checkpoint.snapshot_after_id)?;
    for translation in &page.translations {
        upsert_translation(transaction, scope, &page.pair, translation).await?;
    }
    advance_snapshot(transaction, scope, page, checkpoint.snapshot_after_id).await
}

async fn apply_changes(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
    page: &CorpusChangePageDto,
) -> Result<(), MobileError> {
    let checkpoint = pair_checkpoint(transaction, scope, &page.pair).await?;
    if parse_status(&checkpoint.status)? != PairStatus::Ready {
        return Err(conflict());
    }
    if page.next_cursor == checkpoint.change_cursor {
        return Ok(());
    }
    validate_change_progress(page, checkpoint.change_cursor)?;
    for change in &page.changes {
        match change.kind {
            CorpusChangeKindDto::Upsert => {
                let translation = change.translation.as_ref().ok_or_else(invalid_page)?;
                if translation.translation_id != change.translation_id {
                    return Err(invalid_page());
                }
                validate_translation(translation)?;
                upsert_translation(transaction, scope, &page.pair, translation).await?;
            }
            CorpusChangeKindDto::Delete => {
                if change.translation.is_some() {
                    return Err(invalid_page());
                }
                delete_translation(transaction, scope, &page.pair, change.translation_id).await?;
            }
        }
    }
    advance_change_cursor(transaction, scope, page, checkpoint.change_cursor).await
}

async fn apply_cards(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
    page: &CardChangePageDto,
) -> Result<(), MobileError> {
    ensure_sync_state(transaction, scope).await?;
    let cursor = card_cursor(transaction, scope).await?;
    if page.next_cursor == cursor {
        return Ok(());
    }
    validate_card_progress(page, cursor)?;
    for change in &page.changes {
        match change {
            CardChangeDto::Upsert { sequence, card } => {
                if card.server_cursor != *sequence {
                    return Err(invalid_page());
                }
                validate_card(card)?;
                upsert_card(transaction, scope, card).await?;
            }
            CardChangeDto::Delete { translation_id, .. } => {
                delete_card(transaction, scope, *translation_id).await?;
            }
        }
    }
    advance_card_cursor(transaction, scope, cursor, page.next_cursor).await
}

async fn pair_checkpoint(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
    pair: &LanguagePairDto,
) -> Result<PairCheckpoint, MobileError> {
    sqlx::query_as::<_, PairCheckpoint>(
        "SELECT status, snapshot_watermark, snapshot_after_id, change_cursor
         FROM language_pairs
         WHERE profile_id = ? AND user_id = ? AND native = ? AND \"foreign\" = ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&pair.native_lang)
    .bind(&pair.foreign_lang)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(conflict)
}

async fn upsert_translation(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
    pair: &LanguagePairDto,
    translation: &CorpusTranslationDto,
) -> Result<(), MobileError> {
    validate_translation(translation)?;
    sqlx::query(
        "INSERT INTO translations
             (profile_id, user_id, native, \"foreign\", translation_id,
              from_phrase, to_phrase, frequency, is_phrase)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(profile_id, user_id, native, \"foreign\", translation_id)
         DO UPDATE SET from_phrase = excluded.from_phrase, to_phrase = excluded.to_phrase,
                       frequency = excluded.frequency, is_phrase = excluded.is_phrase",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&pair.native_lang)
    .bind(&pair.foreign_lang)
    .bind(translation.translation_id)
    .bind(&translation.from_phrase)
    .bind(&translation.to_phrase)
    .bind(translation.frequency)
    .bind(translation.is_phrase)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn delete_translation(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
    pair: &LanguagePairDto,
    translation_id: i32,
) -> Result<(), MobileError> {
    if translation_id <= 0 {
        return Err(invalid_page());
    }
    sqlx::query(
        "DELETE FROM translations
         WHERE profile_id = ? AND user_id = ? AND native = ?
           AND \"foreign\" = ? AND translation_id = ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&pair.native_lang)
    .bind(&pair.foreign_lang)
    .bind(translation_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn advance_snapshot(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
    page: &CorpusPageDto,
    previous_cursor: i32,
) -> Result<(), MobileError> {
    let status = if page.has_more {
        "downloading"
    } else {
        "ready"
    };
    let change_cursor = if page.has_more {
        None
    } else {
        Some(page.snapshot_watermark)
    };
    let result = sqlx::query(
        "UPDATE language_pairs
         SET snapshot_after_id = ?, status = ?,
             change_cursor = COALESCE(?, change_cursor), updated_at = ?
         WHERE profile_id = ? AND user_id = ? AND native = ? AND \"foreign\" = ?
           AND snapshot_after_id = ? AND snapshot_watermark = ?",
    )
    .bind(page.next_cursor)
    .bind(status)
    .bind(change_cursor)
    .bind(Utc::now())
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&page.pair.native_lang)
    .bind(&page.pair.foreign_lang)
    .bind(previous_cursor)
    .bind(page.snapshot_watermark)
    .execute(&mut **transaction)
    .await?;
    require_one_row(result.rows_affected())
}

async fn advance_change_cursor(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
    page: &CorpusChangePageDto,
    previous_cursor: i64,
) -> Result<(), MobileError> {
    let result = sqlx::query(
        "UPDATE language_pairs SET change_cursor = ?, updated_at = ?
         WHERE profile_id = ? AND user_id = ? AND native = ? AND \"foreign\" = ?
           AND change_cursor = ?",
    )
    .bind(page.next_cursor)
    .bind(Utc::now())
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&page.pair.native_lang)
    .bind(&page.pair.foreign_lang)
    .bind(previous_cursor)
    .execute(&mut **transaction)
    .await?;
    require_one_row(result.rows_affected())
}

async fn ensure_sync_state(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
) -> Result<(), MobileError> {
    sqlx::query(
        "INSERT OR IGNORE INTO sync_state (profile_id, user_id, card_cursor)
         VALUES (?, ?, 0)",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn card_cursor(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
) -> Result<i64, MobileError> {
    Ok(sqlx::query_scalar(
        "SELECT card_cursor FROM sync_state WHERE profile_id = ? AND user_id = ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .fetch_one(&mut **transaction)
    .await?)
}

async fn upsert_card(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
    card: &CardSnapshotDto,
) -> Result<(), MobileError> {
    sqlx::query(
        "INSERT INTO cards
             (profile_id, user_id, translation_id, stability, difficulty, elapsed_days,
              scheduled_days, reps, lapses, state, last_review, due, server_cursor)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(profile_id, user_id, translation_id) DO UPDATE SET
             stability = excluded.stability, difficulty = excluded.difficulty,
             elapsed_days = excluded.elapsed_days, scheduled_days = excluded.scheduled_days,
             reps = excluded.reps, lapses = excluded.lapses, state = excluded.state,
             last_review = excluded.last_review, due = excluded.due,
             server_cursor = excluded.server_cursor",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(card.translation_id)
    .bind(card.stability)
    .bind(card.difficulty)
    .bind(card.elapsed_days)
    .bind(card.scheduled_days)
    .bind(card.reps)
    .bind(card.lapses)
    .bind(card_state_value(card.state))
    .bind(card.last_review)
    .bind(card.due)
    .bind(card.server_cursor)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn delete_card(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
    translation_id: i32,
) -> Result<(), MobileError> {
    if translation_id <= 0 {
        return Err(invalid_page());
    }
    let pending: bool = sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM review_outbox
             WHERE profile_id = ? AND user_id = ? AND translation_id = ? AND status = 'pending'
         )",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(translation_id)
    .fetch_one(&mut **transaction)
    .await?;
    if pending {
        return Err(MobileError::Conflict(String::from(
            "a pending review blocks the authoritative card deletion",
        )));
    }
    sqlx::query("DELETE FROM cards WHERE profile_id = ? AND user_id = ? AND translation_id = ?")
        .bind(scope.profile_id)
        .bind(scope.user_id)
        .bind(translation_id)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn advance_card_cursor(
    transaction: &mut Transaction<'_, Sqlite>,
    scope: Scope,
    previous_cursor: i64,
    next_cursor: i64,
) -> Result<(), MobileError> {
    let result = sqlx::query(
        "UPDATE sync_state SET card_cursor = ?
         WHERE profile_id = ? AND user_id = ? AND card_cursor = ?",
    )
    .bind(next_cursor)
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(previous_cursor)
    .execute(&mut **transaction)
    .await?;
    require_one_row(result.rows_affected())
}

async fn estimate_row(
    pool: &SqlitePool,
    scope: Scope,
    pair: &LanguagePairDto,
) -> Result<EstimateRow, MobileError> {
    Ok(sqlx::query_as::<_, EstimateRow>(
        "SELECT COUNT(*) AS translation_count,
                COALESCE(SUM(length(CAST(from_phrase AS BLOB))
                           + length(CAST(to_phrase AS BLOB))), 0) AS text_bytes
         FROM translations
         WHERE profile_id = ? AND user_id = ? AND native = ? AND \"foreign\" = ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&pair.native_lang)
    .bind(&pair.foreign_lang)
    .fetch_one(pool)
    .await?)
}

async fn ranked_rows(
    pool: &SqlitePool,
    scope: Scope,
    pair: &LanguagePairDto,
    limit: u16,
) -> Result<Vec<TranslationRow>, MobileError> {
    let words = ranked_rows_by_kind(pool, scope, pair, limit, false).await?;
    let phrases = ranked_rows_by_kind(pool, scope, pair, limit, true).await?;
    Ok(interleave_ranked(words, phrases, usize::from(limit)))
}

async fn ranked_rows_by_kind(
    pool: &SqlitePool,
    scope: Scope,
    pair: &LanguagePairDto,
    limit: u16,
    is_phrase: bool,
) -> Result<Vec<TranslationRow>, MobileError> {
    Ok(sqlx::query_as::<_, TranslationRow>(
        "SELECT translation_id, from_phrase, to_phrase, frequency, is_phrase
         FROM translations
         WHERE profile_id = ? AND user_id = ? AND native = ? AND \"foreign\" = ?
           AND is_phrase = ?
         ORDER BY frequency DESC, translation_id ASC LIMIT ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&pair.native_lang)
    .bind(&pair.foreign_lang)
    .bind(is_phrase)
    .bind(i64::from(limit))
    .fetch_all(pool)
    .await?)
}

async fn translation_row(
    pool: &SqlitePool,
    scope: Scope,
    pair: &LanguagePairDto,
    translation_id: i32,
) -> Result<Option<TranslationRow>, MobileError> {
    Ok(sqlx::query_as::<_, TranslationRow>(
        "SELECT translation_id, from_phrase, to_phrase, frequency, is_phrase
         FROM translations
         WHERE profile_id = ? AND user_id = ? AND native = ?
           AND \"foreign\" = ? AND translation_id = ?",
    )
    .bind(scope.profile_id)
    .bind(scope.user_id)
    .bind(&pair.native_lang)
    .bind(&pair.foreign_lang)
    .bind(translation_id)
    .fetch_optional(pool)
    .await?)
}

fn validate_snapshot_envelope(page: &CorpusPageDto) -> Result<(), MobileError> {
    validate_protocol(page.protocol_version)?;
    validate_pair(&page.pair)?;
    if page.translations.len() > MAX_PAGE_ITEMS
        || page.next_cursor < 0
        || page.snapshot_watermark < 0
    {
        return Err(invalid_page());
    }
    Ok(())
}

fn validate_snapshot_progress(page: &CorpusPageDto, cursor: i32) -> Result<(), MobileError> {
    if page.next_cursor < cursor || (page.has_more && page.next_cursor == cursor) {
        return Err(conflict());
    }
    let mut previous = cursor;
    for translation in &page.translations {
        validate_translation(translation)?;
        if translation.translation_id <= previous || translation.translation_id > page.next_cursor {
            return Err(invalid_page());
        }
        previous = translation.translation_id;
    }
    if previous != page.next_cursor {
        return Err(invalid_page());
    }
    Ok(())
}

fn validate_change_envelope(page: &CorpusChangePageDto) -> Result<(), MobileError> {
    validate_protocol(page.protocol_version)?;
    validate_pair(&page.pair)?;
    if page.changes.len() > MAX_PAGE_ITEMS
        || page.next_cursor < 0
        || page.change_watermark < page.next_cursor
        || (!page.has_more && page.next_cursor != page.change_watermark)
    {
        return Err(invalid_page());
    }
    Ok(())
}

fn validate_change_progress(page: &CorpusChangePageDto, cursor: i64) -> Result<(), MobileError> {
    if page.next_cursor < cursor || (page.has_more && page.next_cursor == cursor) {
        return Err(conflict());
    }
    let mut previous = cursor;
    for change in &page.changes {
        if change.sequence <= previous || change.sequence > page.next_cursor {
            return Err(invalid_page());
        }
        previous = change.sequence;
    }
    if !page.changes.is_empty() && previous != page.next_cursor {
        return Err(invalid_page());
    }
    Ok(())
}

fn validate_card_envelope(page: &CardChangePageDto) -> Result<(), MobileError> {
    validate_protocol(page.protocol_version)?;
    if page.changes.len() > MAX_PAGE_ITEMS
        || page.next_cursor < 0
        || page.change_watermark < page.next_cursor
        || (!page.has_more && page.next_cursor != page.change_watermark)
    {
        return Err(invalid_page());
    }
    Ok(())
}

fn validate_card_progress(page: &CardChangePageDto, cursor: i64) -> Result<(), MobileError> {
    if page.next_cursor < cursor || (page.has_more && page.next_cursor == cursor) {
        return Err(conflict());
    }
    let mut previous = cursor;
    for change in &page.changes {
        let sequence = match change {
            CardChangeDto::Upsert { sequence, .. } | CardChangeDto::Delete { sequence, .. } => {
                *sequence
            }
        };
        if sequence <= previous || sequence > page.next_cursor {
            return Err(invalid_page());
        }
        previous = sequence;
    }
    if !page.changes.is_empty() && previous != page.next_cursor {
        return Err(invalid_page());
    }
    Ok(())
}

fn validate_translation(translation: &CorpusTranslationDto) -> Result<(), MobileError> {
    let valid = translation.translation_id > 0
        && translation.frequency >= 0
        && !translation.from_phrase.is_empty()
        && !translation.to_phrase.is_empty()
        && translation.from_phrase.len() <= MAX_TEXT_BYTES
        && translation.to_phrase.len() <= MAX_TEXT_BYTES
        && !translation.from_phrase.contains('\0')
        && !translation.to_phrase.contains('\0');
    if !valid {
        return Err(invalid_page());
    }
    Ok(())
}

fn validate_card(card: &CardSnapshotDto) -> Result<(), MobileError> {
    let valid = card.translation_id > 0
        && card.stability.is_finite()
        && card.stability >= 0.0
        && card.difficulty.is_finite()
        && card.difficulty >= 0.0
        && card.elapsed_days >= 0
        && card.scheduled_days >= 0
        && card.reps >= 0
        && card.lapses >= 0
        && card.server_cursor >= 0;
    if !valid {
        return Err(invalid_page());
    }
    Ok(())
}

fn validate_pair(pair: &LanguagePairDto) -> Result<(), MobileError> {
    if !valid_language_code(&pair.native_lang)
        || !valid_language_code(&pair.foreign_lang)
        || pair.native_lang == pair.foreign_lang
    {
        return Err(MobileError::InvalidInput(String::from(
            "language pair is invalid",
        )));
    }
    Ok(())
}

fn valid_language_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 32
        && code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn validate_protocol(protocol_version: u16) -> Result<(), MobileError> {
    if protocol_version != MOBILE_PROTOCOL_VERSION {
        return Err(MobileError::UnsupportedProtocol {
            required: MOBILE_PROTOCOL_VERSION,
            actual: protocol_version,
        });
    }
    Ok(())
}

fn validate_limit(limit: u16) -> Result<(), MobileError> {
    if limit == 0 || limit > MAX_QUERY_LIMIT {
        return Err(MobileError::InvalidInput(String::from(
            "query limit is outside the supported range",
        )));
    }
    Ok(())
}

fn card_state_value(state: CardStatusDto) -> i64 {
    match state {
        CardStatusDto::New => 0,
        CardStatusDto::Learning => 1,
        CardStatusDto::Review => 2,
        CardStatusDto::Relearning => 3,
    }
}

fn parse_status(status: &str) -> Result<PairStatus, MobileError> {
    match status {
        "absent" => Ok(PairStatus::Absent),
        "downloading" => Ok(PairStatus::Downloading),
        "ready" => Ok(PairStatus::Ready),
        "failed" => Ok(PairStatus::Failed),
        _ => Err(invalid_state()),
    }
}

fn validate_languages(languages: &[LanguageInfo]) -> Result<(), MobileError> {
    if languages.len() > MAX_PAGE_ITEMS {
        return Err(invalid_page());
    }
    let mut codes = HashSet::with_capacity(languages.len());
    for language in languages {
        let valid_name = !language.name.is_empty()
            && language.name.len() <= 256
            && !language.name.contains('\0')
            && !language.name.chars().any(char::is_control);
        if !valid_language_code(&language.code)
            || !valid_name
            || !codes.insert(language.code.as_str())
        {
            return Err(invalid_page());
        }
    }
    Ok(())
}

fn script_direction_value(direction: ScriptDirection) -> &'static str {
    match direction {
        ScriptDirection::Ltr => "ltr",
        ScriptDirection::Rtl => "rtl",
    }
}

fn sync_error_value(kind: SyncErrorKind) -> &'static str {
    match kind {
        SyncErrorKind::Retryable => "retryable",
        SyncErrorKind::Permanent => "permanent",
        SyncErrorKind::Authentication => "authentication",
    }
}

fn sync_phase_value(phase: SyncPhase) -> &'static str {
    match phase {
        SyncPhase::Idle => "idle",
        SyncPhase::Reviews => "reviews",
        SyncPhase::Nback => "nback",
        SyncPhase::Cards => "cards",
        SyncPhase::Snapshots => "snapshots",
        SyncPhase::Deltas => "deltas",
        SyncPhase::Finishing => "finishing",
    }
}

fn parse_sync_phase(value: &str) -> Result<SyncPhase, MobileError> {
    match value {
        "idle" => Ok(SyncPhase::Idle),
        "reviews" => Ok(SyncPhase::Reviews),
        "nback" => Ok(SyncPhase::Nback),
        "cards" => Ok(SyncPhase::Cards),
        "snapshots" => Ok(SyncPhase::Snapshots),
        "deltas" => Ok(SyncPhase::Deltas),
        "finishing" => Ok(SyncPhase::Finishing),
        _ => Err(invalid_state()),
    }
}

fn optional_i64(value: Option<u64>) -> Result<Option<i64>, MobileError> {
    value
        .map(i64::try_from)
        .transpose()
        .map_err(|_| MobileError::InvalidInput(String::from("estimated byte count is too large")))
}

fn require_one_row(rows_affected: u64) -> Result<(), MobileError> {
    if rows_affected != 1 {
        return Err(conflict());
    }
    Ok(())
}

fn invalid_page() -> MobileError {
    MobileError::InvalidInput(String::from("synchronization page is invalid"))
}

fn conflict() -> MobileError {
    MobileError::Conflict(String::from(
        "synchronization page does not match the local checkpoint",
    ))
}

fn invalid_state() -> MobileError {
    MobileError::Conflict(String::from("local synchronization state is invalid"))
}
