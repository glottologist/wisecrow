pub mod client;

use sqlx::PgPool;
use tracing::info;

use crate::errors::WisecrowError;
use client::SyncClient;

/// Runs a full sync from a remote Wisecrow instance.
///
/// # Errors
///
/// Returns an error if any sync step fails.
pub async fn run_sync(
    pool: &PgPool,
    remote_url: &str,
    api_key: Option<&str>,
) -> Result<(), WisecrowError> {
    let client = SyncClient::new(remote_url, api_key)?;

    info!("Starting sync from {remote_url}");

    let lang_count = client.sync_languages(pool).await?;
    info!("Synced {lang_count} languages");

    let trans_count = client.sync_translations(pool).await?;
    info!("Synced {trans_count} translations");

    let rule_count = client.sync_grammar_rules(pool).await?;
    info!("Synced {rule_count} grammar rules");

    update_sync_metadata(pool, remote_url, "languages", lang_count).await?;
    update_sync_metadata(pool, remote_url, "translations", trans_count).await?;
    update_sync_metadata(pool, remote_url, "grammar_rules", rule_count).await?;

    info!("Sync complete");
    Ok(())
}

async fn update_sync_metadata(
    pool: &PgPool,
    remote_url: &str,
    table_name: &str,
    count: usize,
) -> Result<(), WisecrowError> {
    if count > 0 {
        sqlx::query(
            "INSERT INTO sync_metadata (remote_url, table_name, last_synced_at)
             VALUES ($1, $2, NOW())
             ON CONFLICT (remote_url, table_name)
             DO UPDATE SET last_synced_at = NOW()",
        )
        .bind(remote_url)
        .bind(table_name)
        .execute(pool)
        .await?;
    }
    Ok(())
}
