use anyhow::Result;
pub mod parsing;
pub mod persisting;

use crate::{
    downloader::{self, Downloader},
    errors::WisecrowError,
    files::{LanguageFileInfo, LanguageFiles},
    Langs,
};
use tokio::{
    join,
    sync::{
        mpsc::{channel, Sender},
        RwLock,
    },
    task::JoinHandle,
};
use tracing::{error, info};

const ITEM_QUEUE_BOUND: usize = 1000;
pub struct Ingester {}

impl Ingester {
    pub async fn spawn(langs: Langs, language_file: LanguageFileInfo) -> JoinHandle<()> {
        tokio::spawn(async move {
            if let Err(e) = Self::run(langs.clone(), language_file).await {
                error!("Error ingesting {}: {:?}", langs, e);
            }
        })
    }
    pub async fn run(langs: Langs, language_file: LanguageFileInfo) -> Result<(), WisecrowError> {
        let (item_tx, mut item_rx) = channel::<String>(ITEM_QUEUE_BOUND);
        let downloader = Downloader::new()?;
        let _ = downloader.download(language_file).await?;

        Ok(())
    }
}
