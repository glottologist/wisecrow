use std::future::Future;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use sqlx::PgPool;

use crate::errors::WisecrowError;
use crate::media::fingerprint::MediaFingerprint;
use crate::media::MediaType;

const LOAD_ROW_SQL: &str = "SELECT file_path, attribution, source_fingerprint FROM media_cache
         WHERE translation_id = $1 AND media_type = $2";
const NON_UTF8_CACHE_PATH: &str = "Non-UTF8 cache path";

pub struct MediaCache {
    cache_dir: PathBuf,
    pool: PgPool,
}

struct CacheRow {
    file_path: String,
    attribution: Option<String>,
    source_fingerprint: Option<String>,
}

struct PendingPublish<'a> {
    translation_id: i32,
    media_type: MediaType,
    fingerprint: &'a MediaFingerprint,
    data: &'a [u8],
    attribution: Option<String>,
    previous: Option<PathBuf>,
}

impl CacheRow {
    fn from_tuple(
        (file_path, attribution, source_fingerprint): (String, Option<String>, Option<String>),
    ) -> Self {
        Self {
            file_path,
            attribution,
            source_fingerprint,
        }
    }
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
        Self::with_cache_dir(pool, base)
    }

    /// Creates a media cache rooted at `cache_dir`.
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be created.
    pub fn with_cache_dir(
        pool: PgPool,
        cache_dir: impl AsRef<Path>,
    ) -> Result<Self, WisecrowError> {
        let cache_dir = cache_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(cache_dir.join(MediaType::Audio.as_str()))?;
        std::fs::create_dir_all(cache_dir.join(MediaType::Image.as_str()))?;
        Ok(Self { cache_dir, pool })
    }

    /// Returns the local file path for cached media, fetching via `fetcher`
    /// if no row matches `fingerprint`.
    ///
    /// # Errors
    ///
    /// Returns an error if the fetch, atomic publish, or stale-file cleanup fails.
    pub async fn get_or_fetch<F, Fut>(
        &self,
        translation_id: i32,
        media_type: MediaType,
        fingerprint: &MediaFingerprint,
        fetcher: F,
    ) -> Result<PathBuf, WisecrowError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Vec<u8>, WisecrowError>>,
    {
        self.get_or_fetch_attributed(translation_id, media_type, fingerprint, || async {
            fetcher().await.map(|bytes| (bytes, None))
        })
        .await
        .map(|(path, _)| path)
    }

    /// As [`Self::get_or_fetch`], carrying the credit string a stock provider
    /// returns alongside the bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the fetch, atomic publish, or stale-file cleanup fails.
    pub async fn get_or_fetch_attributed<F, Fut>(
        &self,
        translation_id: i32,
        media_type: MediaType,
        fingerprint: &MediaFingerprint,
        fetcher: F,
    ) -> Result<(PathBuf, Option<String>), WisecrowError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(Vec<u8>, Option<String>), WisecrowError>>,
    {
        if let Some(hit) = self
            .hit_from_pool(translation_id, media_type, fingerprint)
            .await?
        {
            return Ok(hit);
        }

        let mut tx = self.begin_locked(translation_id, media_type).await?;
        let stored = Self::load_row_tx(&mut tx, translation_id, media_type).await?;
        let previous = match stored {
            Some(row) => {
                if let Some(path) = self.matching_live_file(&row, fingerprint) {
                    tx.commit().await?;
                    return Ok((path, row.attribution));
                }
                Some(PathBuf::from(row.file_path))
            }
            None => None,
        };
        let (data, attribution) = fetcher().await?;
        self.publish(
            tx,
            PendingPublish {
                translation_id,
                media_type,
                fingerprint,
                data: &data,
                attribution,
                previous,
            },
        )
        .await
    }

    async fn hit_from_pool(
        &self,
        translation_id: i32,
        media_type: MediaType,
        fingerprint: &MediaFingerprint,
    ) -> Result<Option<(PathBuf, Option<String>)>, WisecrowError> {
        let Some(row) = self.load_row_pool(translation_id, media_type).await? else {
            return Ok(None);
        };
        Ok(self
            .matching_live_file(&row, fingerprint)
            .map(|path| (path, row.attribution)))
    }

    async fn load_row_pool(
        &self,
        translation_id: i32,
        media_type: MediaType,
    ) -> Result<Option<CacheRow>, WisecrowError> {
        let row = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(LOAD_ROW_SQL)
            .bind(translation_id)
            .bind(media_type.as_str())
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(CacheRow::from_tuple))
    }

    async fn load_row_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        translation_id: i32,
        media_type: MediaType,
    ) -> Result<Option<CacheRow>, WisecrowError> {
        let row = sqlx::query_as::<_, (String, Option<String>, Option<String>)>(LOAD_ROW_SQL)
            .bind(translation_id)
            .bind(media_type.as_str())
            .fetch_optional(&mut **tx)
            .await?;
        Ok(row.map(CacheRow::from_tuple))
    }

    async fn begin_locked(
        &self,
        translation_id: i32,
        media_type: MediaType,
    ) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, WisecrowError> {
        let lock_key = (i64::from(translation_id) << 8) | i64::from(media_type.lock_discriminant());
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(lock_key)
            .execute(&mut *tx)
            .await?;
        Ok(tx)
    }

    fn matching_live_file(
        &self,
        row: &CacheRow,
        fingerprint: &MediaFingerprint,
    ) -> Option<PathBuf> {
        if row.source_fingerprint.as_deref() != Some(fingerprint.as_str()) {
            return None;
        }
        let cached = PathBuf::from(&row.file_path);
        self.is_contained_cache_file(&cached).then_some(cached)
    }

    fn fingerprinted_path(
        &self,
        translation_id: i32,
        media_type: MediaType,
        fingerprint: &MediaFingerprint,
    ) -> PathBuf {
        self.cache_dir.join(media_type.as_str()).join(format!(
            "{translation_id}-{}.{}",
            fingerprint.as_str(),
            media_type.extension()
        ))
    }

    async fn publish(
        &self,
        mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
        pending: PendingPublish<'_>,
    ) -> Result<(PathBuf, Option<String>), WisecrowError> {
        let PendingPublish {
            translation_id,
            media_type,
            fingerprint,
            data,
            attribution,
            previous,
        } = pending;
        let final_path = self.fingerprinted_path(translation_id, media_type, fingerprint);
        Self::write_atomic(&final_path, data).await?;
        if let Err(error) = self.remove_tmp_orphans(translation_id, media_type).await {
            tracing::error!(
                translation_id,
                media_type = media_type.as_str(),
                error = %error,
                "failed to remove media cache temp orphans"
            );
        }
        let path_str = utf8_path(&final_path)?;
        Self::upsert_row(
            &mut tx,
            translation_id,
            media_type,
            path_str,
            fingerprint,
            attribution.as_deref(),
        )
        .await?;
        tx.commit().await?;
        self.remove_previous(previous.as_deref(), &final_path)
            .await?;
        Ok((final_path, attribution))
    }

    async fn write_atomic(final_path: &Path, data: &[u8]) -> Result<(), WisecrowError> {
        let temp_path = temp_path_for(final_path)?;
        tokio::fs::write(&temp_path, data).await?;
        if let Err(error) = tokio::fs::rename(&temp_path, final_path).await {
            if let Err(cleanup) = tokio::fs::remove_file(&temp_path).await {
                tracing::error!(
                    path = %temp_path.display(),
                    error = %cleanup,
                    "failed to remove incomplete media cache temp file"
                );
            }
            return Err(error.into());
        }
        Ok(())
    }

    async fn upsert_row(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        translation_id: i32,
        media_type: MediaType,
        file_path: &str,
        fingerprint: &MediaFingerprint,
        attribution: Option<&str>,
    ) -> Result<(), WisecrowError> {
        sqlx::query(
            "INSERT INTO media_cache
                 (translation_id, media_type, file_path, attribution, source_fingerprint)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (translation_id, media_type)
             DO UPDATE SET file_path = $3, attribution = $4, source_fingerprint = $5",
        )
        .bind(translation_id)
        .bind(media_type.as_str())
        .bind(file_path)
        .bind(attribution)
        .bind(fingerprint.as_str())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    async fn remove_previous(
        &self,
        previous: Option<&Path>,
        published: &Path,
    ) -> Result<(), WisecrowError> {
        let Some(previous) = previous else {
            return Ok(());
        };
        if previous == published {
            return Ok(());
        }
        let Some(contained) = self.contained_cache_path(previous) else {
            if previous.exists() {
                tracing::warn!(
                    path = %previous.display(),
                    "refusing to remove media cache path outside cache directory"
                );
            }
            return Ok(());
        };
        match tokio::fs::remove_file(&contained).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => {
                tracing::error!(
                    path = %contained.display(),
                    error = %error,
                    "failed to remove stale media cache file"
                );
                Err(WisecrowError::MediaError(format!(
                    "failed to remove stale cache file {}: {error}",
                    contained.display()
                )))
            }
        }
    }

    async fn remove_tmp_orphans(
        &self,
        translation_id: i32,
        media_type: MediaType,
    ) -> Result<(), std::io::Error> {
        let dir = self.cache_dir.join(media_type.as_str());
        let prefix = format!("{translation_id}-");
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.starts_with(&prefix) && name.ends_with(".tmp") {
                tokio::fs::remove_file(&path).await?;
            }
        }
        Ok(())
    }

    fn is_contained_cache_file(&self, path: &Path) -> bool {
        self.contained_cache_path(path)
            .is_some_and(|canonical| canonical.is_file())
    }

    fn contained_cache_path(&self, path: &Path) -> Option<PathBuf> {
        let root = self.cache_dir.canonicalize().ok()?;
        let canonical = path.canonicalize().ok()?;
        canonical.starts_with(&root).then_some(canonical)
    }
}

fn utf8_path(path: &Path) -> Result<&str, WisecrowError> {
    path.to_str()
        .ok_or_else(|| WisecrowError::InvalidInput(NON_UTF8_CACHE_PATH.to_owned()))
}

fn temp_path_for(final_path: &Path) -> Result<PathBuf, WisecrowError> {
    let file_name = final_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| WisecrowError::InvalidInput(NON_UTF8_CACHE_PATH.to_owned()))?;
    Ok(final_path.with_file_name(format!("{file_name}.{}.tmp", uuid::Uuid::new_v4())))
}
