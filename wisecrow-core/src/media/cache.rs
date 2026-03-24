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
        let file_path = self.file_path(translation_id, media_type);

        let db_row = sqlx::query_as::<_, (String,)>(
            "SELECT file_path FROM media_cache
             WHERE translation_id = $1 AND media_type = $2",
        )
        .bind(translation_id)
        .bind(media_type.as_str())
        .fetch_optional(&self.pool)
        .await?;

        if let Some((cached_path,)) = db_row {
            let cached = PathBuf::from(&cached_path);
            if !cached.starts_with(&self.cache_dir) {
                tracing::warn!("Cached path outside cache directory: {}", cached.display());
            } else if cached.exists() {
                return Ok(cached);
            }
        }

        let data = fetcher().await?;

        let path_str = file_path
            .to_str()
            .ok_or_else(|| WisecrowError::InvalidInput("Non-UTF8 cache path".to_owned()))?;

        tokio::fs::write(&file_path, &data).await?;

        sqlx::query(
            "INSERT INTO media_cache (translation_id, media_type, file_path)
             VALUES ($1, $2, $3)
             ON CONFLICT (translation_id, media_type)
             DO UPDATE SET file_path = $3",
        )
        .bind(translation_id)
        .bind(media_type.as_str())
        .bind(path_str)
        .execute(&self.pool)
        .await?;

        Ok(file_path)
    }

    fn file_path(&self, translation_id: i32, media_type: MediaType) -> PathBuf {
        self.cache_dir
            .join(media_type.as_str())
            .join(format!("{translation_id}.{}", media_type.extension()))
    }
}
