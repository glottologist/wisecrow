use crate::errors::WisecrowError;
use crate::files::{Compression, LanguageFileInfo};
use flate2::read::GzDecoder;
use indicatif::{ProgressBar, ProgressStyle};
use reqwest::Client;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::time::Duration;
use url::Url;
use zip::read::ZipArchive;

const MAX_FILE_SIZE_OVERFLOW_MSG: &str = "max_file_size_mb overflow";
const MAX_DECOMPRESSED_OVERFLOW_MSG: &str = "max_decompressed_mb overflow";
const CONNECT_TIMEOUT_SECS: u64 = 30;

#[derive(Clone, Copy)]
pub struct DownloadConfig {
    pub max_retries: u32,
    pub timeout_seconds: u64,
    pub max_file_size_mb: u64,
    /// Ceiling on the decompressed size of a single archive. This guards
    /// against decompression bombs, so it must stay bounded, but corpus
    /// releases are legitimately large: the NLLB Irish TMX expands to roughly
    /// 5 GB. The default leaves room for those while still capping the damage
    /// a hostile archive can do.
    pub max_decompressed_mb: u64,
    pub unpack: bool,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            timeout_seconds: 300,
            max_file_size_mb: 102_400,
            max_decompressed_mb: 8192,
            unpack: true,
        }
    }
}

pub struct Downloader {
    config: DownloadConfig,
    client: Client,
}

impl Downloader {
    /// Creates a new `Downloader` with a shared HTTP client built from `config`.
    ///
    /// # Errors
    ///
    /// Returns [`WisecrowError::UnableToGetFile`] if the HTTP client cannot be
    /// constructed (e.g., TLS initialisation failure).
    pub fn new(config: DownloadConfig) -> Result<Self, WisecrowError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .read_timeout(Duration::from_secs(config.timeout_seconds))
            .build()?;
        Ok(Self { config, client })
    }

    fn unzip(zip_path: &str, output_dir: &str) -> io::Result<()> {
        let file = File::open(zip_path)?;
        let mut archive = ZipArchive::new(file)?;
        let output_path = Path::new(output_dir);
        std::fs::create_dir_all(output_path)?;
        let canonical_root = output_path.canonicalize()?;

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let name = entry.name().to_owned();

            if name.contains("..") || name.starts_with('/') || name.starts_with('\\') {
                tracing::warn!("Skipping suspicious path in ZIP: {name}");
                continue;
            }

            let outpath = canonical_root.join(&name);

            if !outpath.starts_with(&canonical_root) {
                tracing::warn!("Skipping path that escapes extraction root: {name}");
                continue;
            }

            if entry.is_dir() {
                std::fs::create_dir_all(&outpath)?;
            } else {
                if let Some(parent) = outpath.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut outfile = File::create(&outpath)?;
                io::copy(&mut entry, &mut outfile)?;
            }
        }
        Ok(())
    }

    fn decompress_gz(input_path: &str, output_path: &str, max_bytes: u64) -> io::Result<()> {
        let input_file = File::open(input_path)?;
        let decoder = GzDecoder::new(BufReader::new(input_file));
        let mut limited = decoder.take(max_bytes.saturating_add(1));
        let output_file = File::create(output_path)?;
        let mut buffered_output = BufWriter::new(output_file);
        let written = io::copy(&mut limited, &mut buffered_output)?;
        if written > max_bytes {
            drop(buffered_output);
            std::fs::remove_file(output_path).ok();
            return Err(io::Error::other(format!(
                "Decompressed output exceeds the {max_bytes} byte limit; raise --max-decompressed-mb to ingest this archive"
            )));
        }
        std::fs::remove_file(input_path)
    }

    /// Downloads `file`, retrying up to `config.max_retries` times with
    /// exponential back-off.
    ///
    /// # Errors
    ///
    /// Returns an error if all retry attempts fail, the server returns a
    /// non-success status, or the response body exceeds `max_file_size_mb`.
    pub async fn download(&self, file: &LanguageFileInfo) -> Result<String, WisecrowError> {
        self.download_to(file, None).await
    }

    pub async fn download_to(
        &self,
        file: &LanguageFileInfo,
        output_dir: Option<&Path>,
    ) -> Result<String, WisecrowError> {
        let mut last_err = None;
        for attempt in 0..=self.config.max_retries {
            match self.try_download(file, output_dir).await {
                Ok(path) => return Ok(path),
                Err(e) => {
                    if Self::is_permanent(&e) {
                        tracing::warn!("Download of {} failed permanently: {e}", file.file_name);
                        last_err = Some(e);
                        break;
                    }
                    if attempt < self.config.max_retries {
                        let delay = Duration::from_secs(2u64.pow(attempt));
                        tracing::warn!(
                            "Download attempt {} failed: {e}. Retrying in {delay:?}",
                            attempt.saturating_add(1),
                        );
                        tokio::time::sleep(delay).await;
                    }
                    last_err = Some(e);
                }
            }
        }
        let file_path = Self::resolve_path(&file.file_name, output_dir);
        if let Err(e) = std::fs::remove_file(&file_path) {
            tracing::warn!("Failed to clean up partial download {file_path}: {e}");
        }
        Err(last_err.unwrap_or(WisecrowError::DownloadRetriesExhausted))
    }

    /// Returns `true` for failures that repeating the request cannot resolve.
    /// A missing corpus release is the common case: OPUS publishes no file for
    /// most language pairs in most collections, and retrying a 404 four times
    /// with back-off merely delays the inevitable. Timeouts and rate limits are
    /// deliberately excluded, since those do clear on their own.
    fn is_permanent(error: &WisecrowError) -> bool {
        match error {
            WisecrowError::HttpStatus { status, .. } => {
                (400..500).contains(status) && *status != 408 && *status != 429
            }
            WisecrowError::FileTooLarge(_) => true,
            _ => false,
        }
    }

    fn resolve_path(name: &str, output_dir: Option<&Path>) -> String {
        match output_dir {
            Some(dir) => dir.join(name).to_string_lossy().into_owned(),
            None => name.to_owned(),
        }
    }

    async fn try_download(
        &self,
        file: &LanguageFileInfo,
        output_dir: Option<&Path>,
    ) -> Result<String, WisecrowError> {
        tracing::info!(
            "Downloading {} from {}",
            file.file_name,
            file.target_location
        );

        let url = Url::parse(&file.target_location)?;
        let response = self.client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(WisecrowError::HttpStatus {
                status: response.status().as_u16(),
                url: file.target_location.clone(), // clone: the error outlives the borrow
            });
        }

        let content_length = response.content_length();
        self.check_file_size(content_length)?;

        let progress_bar = ProgressBar::new(content_length.unwrap_or(0));
        let style = ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")?
            .progress_chars("#>-");
        progress_bar.set_style(style);

        let file_path = Self::resolve_path(&file.file_name, output_dir);
        self.stream_to_file(&file_path, response, &progress_bar)
            .await?;
        progress_bar.finish_with_message("Download completed!");

        self.decompress_if_needed(file, output_dir)
    }

    fn check_file_size(&self, content_length: Option<u64>) -> Result<(), WisecrowError> {
        if let Some(size) = content_length {
            let max_bytes = self
                .config
                .max_file_size_mb
                .checked_mul(1024 * 1024)
                .ok_or_else(|| {
                    WisecrowError::InvalidInput(MAX_FILE_SIZE_OVERFLOW_MSG.to_owned())
                })?;
            if size > max_bytes {
                return Err(WisecrowError::FileTooLarge(format!(
                    "{size} bytes (max: {max_bytes} bytes)"
                )));
            }
        }
        Ok(())
    }

    async fn stream_to_file(
        &self,
        path: &str,
        mut response: reqwest::Response,
        progress_bar: &ProgressBar,
    ) -> Result<(), WisecrowError> {
        let max_bytes = self
            .config
            .max_file_size_mb
            .checked_mul(1024 * 1024)
            .ok_or_else(|| WisecrowError::InvalidInput(MAX_FILE_SIZE_OVERFLOW_MSG.to_owned()))?;
        let mut fileio = BufWriter::new(File::create(path)?);
        let mut downloaded: u64 = 0;

        while let Some(chunk) = response.chunk().await? {
            fileio.write_all(&chunk)?;
            let chunk_len = u64::try_from(chunk.len()).map_err(|_| {
                WisecrowError::InvalidInput("Chunk size exceeds u64 range".to_string())
            })?;
            downloaded = downloaded.saturating_add(chunk_len);
            if downloaded > max_bytes {
                return Err(WisecrowError::FileTooLarge(format!(
                    "response body exceeds the maximum of {max_bytes} bytes"
                )));
            }
            progress_bar.set_position(downloaded);
        }
        Ok(())
    }

    fn decompress_if_needed(
        &self,
        file: &LanguageFileInfo,
        output_dir: Option<&Path>,
    ) -> Result<String, WisecrowError> {
        let compressed_path = Self::resolve_path(&file.file_name, output_dir);
        let decompressed_name = file.decompressed_name();
        let output_path = Self::resolve_path(&decompressed_name, output_dir);
        if self.config.unpack {
            match file.compressed {
                Compression::GzCompressed => {
                    let max_bytes = self
                        .config
                        .max_decompressed_mb
                        .checked_mul(1024 * 1024)
                        .ok_or_else(|| {
                            WisecrowError::InvalidInput(MAX_DECOMPRESSED_OVERFLOW_MSG.to_owned())
                        })?;
                    Self::decompress_gz(&compressed_path, &output_path, max_bytes)?;
                }
                Compression::ZipCompressed => {
                    Self::unzip(&compressed_path, &output_path)?;
                }
            }
        }
        Ok(output_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression as GzLevel;
    use rstest::rstest;

    fn write_gz(dir: &Path, name: &str, payload: &[u8]) -> String {
        let path = dir.join(name);
        let mut encoder = GzEncoder::new(File::create(&path).unwrap(), GzLevel::default());
        encoder.write_all(payload).unwrap();
        encoder.finish().unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn decompresses_archive_within_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let payload = vec![b'a'; 4096];
        let input = write_gz(dir.path(), "corpus.tmx.gz", &payload);
        let output = dir.path().join("corpus.tmx").to_string_lossy().into_owned();

        Downloader::decompress_gz(&input, &output, 8192).unwrap();

        assert_eq!(std::fs::read(&output).unwrap(), payload);
        assert!(
            !Path::new(&input).exists(),
            "the archive should be removed once expanded"
        );
    }

    #[test]
    fn rejects_archive_exceeding_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let input = write_gz(dir.path(), "bomb.tmx.gz", &vec![b'a'; 4096]);
        let output = dir.path().join("bomb.tmx").to_string_lossy().into_owned();

        let err = Downloader::decompress_gz(&input, &output, 1024).unwrap_err();

        assert!(err.to_string().contains("exceeds the 1024 byte limit"));
        assert!(
            !Path::new(&output).exists(),
            "the partial expansion should be cleaned up"
        );
        assert!(
            Path::new(&input).exists(),
            "a rejected archive should be left for the caller to clean up"
        );
    }

    #[rstest]
    #[case(404, true)]
    #[case(403, true)]
    #[case(400, true)]
    #[case(408, false)]
    #[case(429, false)]
    #[case(500, false)]
    #[case(503, false)]
    fn client_errors_are_permanent_except_timeout_and_rate_limit(
        #[case] status: u16,
        #[case] expected: bool,
    ) {
        let error = WisecrowError::HttpStatus {
            status,
            url: "https://example.com/corpus.tmx.gz".to_owned(),
        };
        assert_eq!(Downloader::is_permanent(&error), expected);
    }

    #[test]
    fn size_rejections_are_permanent_and_transport_faults_are_not() {
        assert!(Downloader::is_permanent(&WisecrowError::FileTooLarge(
            "1 bytes (max: 0 bytes)".to_owned()
        )));
        assert!(!Downloader::is_permanent(
            &WisecrowError::DownloadRetriesExhausted
        ));
    }

    #[test]
    fn default_limit_accommodates_the_largest_corpus_releases() {
        // The NLLB Irish TMX expands to roughly 5 GB; a default below that
        // silently discards the corpus after a multi-gigabyte download.
        let bytes = DownloadConfig::default()
            .max_decompressed_mb
            .checked_mul(1024 * 1024)
            .unwrap();
        assert!(bytes >= 6 * 1024 * 1024 * 1024);
    }
}
