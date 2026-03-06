use crate::errors::WisecrowError;
use crate::ingesting::parsing::TranslationPair;
use sqlx::PgPool;
use tokio::sync::mpsc::Receiver;

const TRANSLATION_BATCH_SIZE: usize = 1000;

pub struct DatabasePersister {
    pool: PgPool,
}

impl DatabasePersister {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Upserts a language row and returns its id.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub async fn ensure_language(&self, code: &str, name: &str) -> Result<i32, WisecrowError> {
        let row = sqlx::query_scalar::<_, i32>(
            "INSERT INTO languages (code, name) VALUES ($1, $2)
             ON CONFLICT (code) DO UPDATE SET code = EXCLUDED.code
             RETURNING id",
        )
        .bind(code)
        .bind(name)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Inserts `batch` as a single bulk statement using `unnest`.
    ///
    /// # Errors
    ///
    /// Returns an error if the database transaction fails.
    pub async fn persist_translations(
        &self,
        batch: &[TranslationPair],
        from_lang_id: i32,
        to_lang_id: i32,
    ) -> Result<(), WisecrowError> {
        if batch.is_empty() {
            return Ok(());
        }

        let mut seen = std::collections::HashSet::new();
        let deduped: Vec<&TranslationPair> = batch
            .iter()
            .filter(|p| seen.insert((&p.source_text, &p.target_text)))
            .collect();

        let sources: Vec<&str> = deduped.iter().map(|p| p.source_text.as_str()).collect();
        let targets: Vec<&str> = deduped.iter().map(|p| p.target_text.as_str()).collect();

        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO translations (from_language_id, from_phrase, to_language_id, to_phrase)
             SELECT $1, phrase, $3, target_phrase
             FROM unnest($2::text[], $4::text[]) AS t(phrase, target_phrase)
             ON CONFLICT (from_language_id, from_phrase, to_language_id, to_phrase)
             DO UPDATE SET frequency = translations.frequency + 1",
        )
        .bind(from_lang_id)
        .bind(sources)
        .bind(to_lang_id)
        .bind(targets)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn maybe_flush_translations(
        &self,
        batch: &mut Vec<TranslationPair>,
        from_lang_id: i32,
        to_lang_id: i32,
    ) -> Result<(), WisecrowError> {
        if batch.len() >= TRANSLATION_BATCH_SIZE {
            self.persist_translations(batch, from_lang_id, to_lang_id)
                .await?;
            tracing::info!("Persisted {} translations", batch.len());
            batch.clear();
        }
        Ok(())
    }

    /// Drains `receiver`, persisting items in batches until the channel closes.
    ///
    /// # Errors
    ///
    /// Returns an error if any batch persistence fails.
    pub async fn consume(
        &self,
        mut receiver: Receiver<TranslationPair>,
        from_lang_id: i32,
        to_lang_id: i32,
    ) -> Result<(), WisecrowError> {
        let mut translation_batch: Vec<TranslationPair> = Vec::new();

        while let Some(pair) = receiver.recv().await {
            translation_batch.push(pair);
            self.maybe_flush_translations(&mut translation_batch, from_lang_id, to_lang_id)
                .await?;
        }

        if !translation_batch.is_empty() {
            self.persist_translations(&translation_batch, from_lang_id, to_lang_id)
                .await?;
        }

        Ok(())
    }
}
