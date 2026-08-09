use std::future::Future;
use std::path::PathBuf;

use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::media::MediaType;

pub struct MediaCache {
    cache_dir: PathBuf,
    pool: PgPool,
}

impl MediaCache {
    /// Creates a new media cache, initialising the cache directory structure.
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be created.
    pub fn new(pool: PgPool) -> Result<Self, WisecrowError> {
        let base = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("wisecrow")
            .join("cache");

        std::fs::create_dir_all(base.join("audio"))?;
        std::fs::create_dir_all(base.join("image"))?;

        Ok(Self {
            cache_dir: base,
            pool,
        })
    }

    /// Returns the local file path for cached media, fetching via `fetcher`
    /// if not already cached.
    ///
    /// # Errors
    ///
    /// Returns an error if the fetch or file write fails.
    pub async fn get_or_fetch<F, Fut>(
        &self,
        translation_id: i32,
        media_type: MediaType,
        fetcher: F,
    ) -> Result<PathBuf, WisecrowError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<u8>, WisecrowError>>,
    {
        self.get_or_fetch_attributed(translation_id, media_type, || async {
            fetcher().await.map(|bytes| (bytes, None))
        })
        .await
        .map(|(path, _)| path)
    }

    /// As [`Self::get_or_fetch`], carrying the credit string a stock provider
    /// returns alongside the bytes.
    ///
    /// The attribution is stored with the cache row, because the licence
    /// requires it on every display and a cache hit never calls the fetcher
    /// again.
    ///
    /// # Errors
    ///
    /// Returns an error if the fetch or file write fails.
    pub async fn get_or_fetch_attributed<F, Fut>(
        &self,
        translation_id: i32,
        media_type: MediaType,
        fetcher: F,
    ) -> Result<(PathBuf, Option<String>), WisecrowError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(Vec<u8>, Option<String>), WisecrowError>>,
    {
        let file_path = self.file_path(translation_id, media_type);

        if let Some(hit) = self
            .cached_row(translation_id, media_type, &self.pool)
            .await?
        {
            return Ok(hit);
        }

        // Serialise concurrent misses per (translation, type): the second
        // holder finds the row the first wrote and returns without spending
        // provider quota. Key layout: translation_id in the high bits, media
        // type in the low byte.
        let lock_key = (i64::from(translation_id) << 8) | i64::from(media_type.lock_discriminant());
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut *tx)
            .await?;

        let recheck = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT file_path, attribution FROM media_cache
             WHERE translation_id = $1 AND media_type = $2",
        )
        .bind(translation_id)
        .bind(media_type.as_str())
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((cached_path, attribution)) = recheck {
            let cached = PathBuf::from(&cached_path);
            if cached.starts_with(&self.cache_dir) && cached.exists() {
                tx.commit().await?;
                return Ok((cached, attribution));
            }
        }

        let (data, attribution) = fetcher().await?;

        let path_str = file_path
            .to_str()
            .ok_or_else(|| WisecrowError::InvalidInput("Non-UTF8 cache path".to_owned()))?;

        tokio::fs::write(&file_path, &data).await?;

        sqlx::query(
            "INSERT INTO media_cache (translation_id, media_type, file_path, attribution)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (translation_id, media_type)
             DO UPDATE SET file_path = $3, attribution = $4",
        )
        .bind(translation_id)
        .bind(media_type.as_str())
        .bind(path_str)
        .bind(attribution.as_deref())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        Ok((file_path, attribution))
    }

    /// Lock-free read of the cache row; the fast path for hits.
    async fn cached_row(
        &self,
        translation_id: i32,
        media_type: MediaType,
        pool: &PgPool,
    ) -> Result<Option<(PathBuf, Option<String>)>, WisecrowError> {
        let db_row = sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT file_path, attribution FROM media_cache
             WHERE translation_id = $1 AND media_type = $2",
        )
        .bind(translation_id)
        .bind(media_type.as_str())
        .fetch_optional(pool)
        .await?;

        if let Some((cached_path, attribution)) = db_row {
            let cached = PathBuf::from(&cached_path);
            if !cached.starts_with(&self.cache_dir) {
                tracing::warn!("Cached path outside cache directory: {}", cached.display());
            } else if cached.exists() {
                return Ok(Some((cached, attribution)));
            }
        }
        Ok(None)
    }

    fn file_path(&self, translation_id: i32, media_type: MediaType) -> PathBuf {
        self.cache_dir
            .join(media_type.as_str())
            .join(format!("{translation_id}.{}", media_type.extension()))
    }
}
