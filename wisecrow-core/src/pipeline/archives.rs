//! Which OPUS archives have been read to the end, or found to be absent.

use crate::errors::WisecrowError;
use sqlx::PgPool;

pub struct ArchiveLedger<'a> {
    pool: &'a PgPool,
}

impl<'a> ArchiveLedger<'a> {
    #[must_use]
    pub const fn new(pool: &'a PgPool) -> Self {
        Self { pool }
    }

    /// # Errors
    ///
    /// Returns an error when the marker cannot be written.
    pub async fn mark_unavailable(
        &self,
        native: &str,
        foreign: &str,
        corpus: &str,
        archive_url: &str,
    ) -> Result<(), WisecrowError> {
        self.record(native, foreign, corpus, archive_url, "unavailable")
            .await
    }

    /// # Errors
    ///
    /// Returns an error when the marker cannot be written.
    pub async fn mark_ingested(
        &self,
        native: &str,
        foreign: &str,
        corpus: &str,
        archive_url: &str,
    ) -> Result<(), WisecrowError> {
        self.record(native, foreign, corpus, archive_url, "ingested")
            .await
    }

    /// `true` when no marker exists for this archive.
    ///
    /// # Errors
    ///
    /// Returns an error when the lookup fails.
    pub async fn needs_download(
        &self,
        native: &str,
        foreign: &str,
        archive_url: &str,
    ) -> Result<bool, WisecrowError> {
        Ok(!self.is_settled(native, foreign, archive_url).await?)
    }

    /// `true` when this archive was ingested or recorded as unavailable.
    ///
    /// # Errors
    ///
    /// Returns an error when the lookup fails.
    pub async fn is_settled(
        &self,
        native: &str,
        foreign: &str,
        archive_url: &str,
    ) -> Result<bool, WisecrowError> {
        let settled = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (
                 SELECT 1 FROM pipeline_archives
                 WHERE native_code = $1 AND foreign_code = $2 AND archive_url = $3
             )",
        )
        .bind(native)
        .bind(foreign)
        .bind(archive_url)
        .fetch_one(self.pool)
        .await?;
        Ok(settled)
    }

    async fn record(
        &self,
        native: &str,
        foreign: &str,
        corpus: &str,
        archive_url: &str,
        status: &str,
    ) -> Result<(), WisecrowError> {
        sqlx::query(
            "INSERT INTO pipeline_archives
                 (native_code, foreign_code, corpus, archive_url, status)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (native_code, foreign_code, archive_url) DO NOTHING",
        )
        .bind(native)
        .bind(foreign)
        .bind(corpus)
        .bind(archive_url)
        .bind(status)
        .execute(self.pool)
        .await?;
        Ok(())
    }
}
