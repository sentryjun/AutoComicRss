// SPDX-License-Identifier: GPL-2.0-only
use crate::{
    domain::*,
    error::{Error, Result},
};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    SqlitePool,
};
use std::{
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct Store {
    pub pool: SqlitePool,
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

impl Store {
    pub async fn active_downloader_id(&self) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar(
                "SELECT downloader_id FROM downloader_selection WHERE singleton = 1",
            )
            .fetch_one(&self.pool)
            .await?,
        )
    }

    pub async fn downloaders(&self) -> Result<Vec<DownloaderView>> {
        let rows: Vec<(String, String, String, bool)> = sqlx::query_as(
            "SELECT d.id, d.name, d.config, COALESCE(s.downloader_id = d.id, 0) FROM downloader_instances d CROSS JOIN downloader_selection s ORDER BY d.rowid")
            .fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|(id, name, json, is_active)| {
                let config: crate::config::QbitConfig = serde_json::from_str(&json)?;
                Ok(DownloaderView {
                    id,
                    name,
                    kind: DownloaderKind::Qbittorrent,
                    url: config.url,
                    username: config.username,
                    save_path: config.save_path,
                    password_configured: !config.password.is_empty(),
                    is_active,
                })
            })
            .collect()
    }

    pub async fn create_downloader(&self, mut input: DownloaderInput) -> Result<DownloaderView> {
        input.validate()?;
        let id = uuid::Uuid::new_v4().to_string();
        let result = sqlx::query("INSERT INTO downloader_instances(id, name, kind, endpoint, config) VALUES (?, ?, 'qbittorrent', ?, ?) ON CONFLICT(endpoint) DO NOTHING")
            .bind(&id).bind(&input.name).bind(&input.connection.url)
            .bind(serde_json::to_string(&input.connection)?).execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            return Err(Error::Invalid(
                "this downloader endpoint is already registered".into(),
            ));
        }
        self.downloaders()
            .await?
            .into_iter()
            .find(|d| d.id == id)
            .ok_or(Error::NotFound)
    }

    pub(crate) async fn downloader_config(&self, id: &str) -> Result<crate::config::QbitConfig> {
        let json: String =
            sqlx::query_scalar("SELECT config FROM downloader_instances WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?
                .ok_or(Error::NotFound)?;
        Ok(serde_json::from_str(&json)?)
    }

    pub async fn activate_downloader(&self, id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM downloader_instances WHERE id = ?)")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        if !exists {
            return Err(Error::NotFound);
        }
        let destination: Option<String> = sqlx::query_scalar(
            "SELECT destination_id FROM destination_selection WHERE singleton = 1",
        )
        .fetch_one(&mut *tx)
        .await?;
        if let Some(destination) = destination {
            let mapped: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM destination_mappings WHERE destination_id = ? AND downloader_id = ?)")
                .bind(destination).bind(id).fetch_one(&mut *tx).await?;
            if !mapped {
                return Err(Error::Invalid("configure a mapping to the active destination before selecting this downloader".into()));
            }
        }
        sqlx::query("UPDATE downloader_selection SET downloader_id = ? WHERE singleton = 1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        // Bind only new, never-owned work. A switch never rewrites an existing owner.
        sqlx::query("UPDATE download_jobs SET downloader_id = ? WHERE downloader_id IS NULL AND legacy_unassigned = 0")
            .bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn claim_legacy_jobs(&self, id: &str) -> Result<u64> {
        self.downloader_config(id).await?;
        Ok(sqlx::query("UPDATE download_jobs SET downloader_id = ?, legacy_unassigned = 0 WHERE downloader_id IS NULL AND legacy_unassigned = 1")
            .bind(id).execute(&self.pool).await?.rows_affected())
    }

    pub async fn legacy_unassigned_count(&self) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT COUNT(*) FROM download_jobs WHERE legacy_unassigned = 1 AND downloader_id IS NULL")
            .fetch_one(&self.pool).await?)
    }

    pub(crate) async fn bootstrap_downloader(
        &self,
        config: Option<&crate::config::QbitConfig>,
    ) -> Result<()> {
        let Some(config) = config else {
            return Ok(());
        };
        let mut config = config.clone();
        config.validate()?;
        let mut tx = self.pool.begin().await?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM downloader_instances")
            .fetch_one(&mut *tx)
            .await?;
        if count == 0 {
            let id = uuid::Uuid::new_v4().to_string();
            sqlx::query("INSERT INTO downloader_instances(id, name, kind, endpoint, config) VALUES (?, 'Initial qBittorrent', 'qbittorrent', ?, ?)")
                .bind(&id).bind(&config.url).bind(serde_json::to_string(&config)?).execute(&mut *tx).await?;
            sqlx::query("UPDATE downloader_selection SET downloader_id = ? WHERE singleton = 1")
                .bind(&id)
                .execute(&mut *tx)
                .await?;
            // Initial import uses the operator-supplied original QBIT_* connection.
            sqlx::query("UPDATE download_jobs SET downloader_id = ?, legacy_unassigned = 0 WHERE downloader_id IS NULL")
                .bind(&id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn open(url: &str) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(url)?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        // A single connection also makes sqlite::memory: deterministic in tests.
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        sqlx::migrate!().run(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn subscriptions(&self) -> Result<Vec<Subscription>> {
        let rows = sqlx::query_as::<_, (String, String, Option<i64>, Option<String>)>(
            "SELECT id, spec, last_checked_at, last_error FROM subscriptions WHERE deleted = 0 ORDER BY rowid"
        ).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|(id, spec, last_checked_at, last_error)| {
                Ok(Subscription {
                    id,
                    input: serde_json::from_str(&spec)?,
                    last_checked_at,
                    last_error,
                })
            })
            .collect()
    }

    pub async fn subscription(&self, id: &str) -> Result<Subscription> {
        self.subscriptions()
            .await?
            .into_iter()
            .find(|s| s.id == id)
            .ok_or(Error::NotFound)
    }

    pub async fn create(&self, mut input: SubscriptionInput) -> Result<Subscription> {
        input.validate()?;
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO subscriptions(id, spec) VALUES (?, ?)")
            .bind(&id)
            .bind(serde_json::to_string(&input)?)
            .execute(&self.pool)
            .await?;
        self.subscription(&id).await
    }

    pub async fn update(&self, id: &str, mut input: SubscriptionInput) -> Result<Subscription> {
        input.validate()?;
        let result = sqlx::query("UPDATE subscriptions SET spec = ? WHERE id = ? AND deleted = 0")
            .bind(serde_json::to_string(&input)?)
            .bind(id)
            .execute(&self.pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound);
        }
        self.subscription(id).await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let result =
            sqlx::query("UPDATE subscriptions SET deleted = 1 WHERE id = ? AND deleted = 0")
                .bind(id)
                .execute(&self.pool)
                .await?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    pub async fn checked(&self, id: &str, error: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE subscriptions SET last_checked_at = ?, last_error = ? WHERE id = ?")
            .bind(now())
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn already_queued(&self, id: &str, season: i32, release: &Release) -> Result<bool> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM download_jobs WHERE subscription_id = ? AND (resource_key = ? OR (season = ? AND episode = ?))")
            .bind(id).bind(&release.key).bind(season).bind(release.episode).fetch_one(&self.pool).await?;
        Ok(count > 0)
    }

    pub async fn record_scan(&self, sub: &Subscription, releases: &[Release]) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let downloader_id: Option<String> = sqlx::query_scalar(
            "SELECT downloader_id FROM downloader_selection WHERE singleton = 1",
        )
        .fetch_one(&mut *tx)
        .await?;
        let destination_id: Option<String> = sqlx::query_scalar(
            "SELECT destination_id FROM destination_selection WHERE singleton = 1",
        )
        .fetch_one(&mut *tx)
        .await?;
        let mut mapping_id = None;
        let mut download_save_path = None;
        if let Some(destination) = &destination_id {
            let mapping: crate::media::Mapping = sqlx::query_as("SELECT * FROM destination_mappings WHERE destination_id = ? AND downloader_id = ? ORDER BY version DESC LIMIT 1")
                .bind(destination).bind(&downloader_id).fetch_optional(&mut *tx).await?
                .ok_or_else(|| Error::Invalid("active destination has no mapping for the selected downloader".into()))?;
            mapping_id = Some(mapping.id);
            download_save_path = Some(mapping.download_root);
        } else if let Some(downloader) = &downloader_id {
            let config: String =
                sqlx::query_scalar("SELECT config FROM downloader_instances WHERE id = ?")
                    .bind(downloader)
                    .fetch_one(&mut *tx)
                    .await?;
            download_save_path =
                Some(serde_json::from_str::<crate::config::QbitConfig>(&config)?.save_path);
        }
        let mut queued = 0;
        for release in releases {
            sqlx::query("INSERT INTO releases(subscription_id, resource_key, payload) VALUES (?, ?, ?) ON CONFLICT(subscription_id, resource_key) DO UPDATE SET payload = excluded.payload")
                .bind(&sub.id).bind(&release.key).bind(serde_json::to_string(release)?).execute(&mut *tx).await?;
            if release.decision == Decision::Ready {
                if let (Some(episode), Some(url)) = (release.episode, &release.download_url) {
                    let result = sqlx::query("INSERT OR IGNORE INTO download_jobs(id, subscription_id, resource_key, title, season, episode, download_url, downloader_id, destination_id, mapping_id, download_save_path) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
                        .bind(uuid::Uuid::new_v4().to_string()).bind(&sub.id).bind(&release.key)
                        .bind(&release.title).bind(sub.input.season).bind(episode).bind(url).bind(&downloader_id)
                        .bind(&destination_id).bind(&mapping_id).bind(&download_save_path).execute(&mut *tx).await?;
                    queued += result.rows_affected() as usize;
                }
            }
        }
        tx.commit().await?;
        Ok(queued)
    }

    pub async fn releases(&self, id: &str) -> Result<Vec<Release>> {
        self.subscription(id).await?;
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT payload FROM releases WHERE subscription_id = ? ORDER BY rowid DESC LIMIT 500",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|s| serde_json::from_str(&s).map_err(Error::from))
            .collect()
    }

    pub async fn jobs(&self) -> Result<Vec<DownloadJob>> {
        Ok(
            sqlx::query_as("SELECT * FROM download_jobs ORDER BY rowid DESC LIMIT 500")
                .fetch_all(&self.pool)
                .await?,
        )
    }

    pub async fn active_jobs(&self) -> Result<Vec<DownloadJob>> {
        Ok(sqlx::query_as("SELECT * FROM download_jobs WHERE state NOT IN ('completed', 'missing', 'failed') AND next_attempt_at <= ? ORDER BY rowid")
            .bind(now()).fetch_all(&self.pool).await?)
    }

    pub async fn prepare(&self, id: &str, hash: &str, bytes: Option<&[u8]>) -> Result<()> {
        sqlx::query("UPDATE download_jobs SET info_hash = ?, torrent_bytes = ? WHERE id = ?")
            .bind(hash)
            .bind(bytes)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn torrent_bytes(&self, id: &str) -> Result<Option<Vec<u8>>> {
        Ok(
            sqlx::query_scalar("SELECT torrent_bytes FROM download_jobs WHERE id = ?")
                .bind(id)
                .fetch_one(&self.pool)
                .await?,
        )
    }

    pub async fn state(&self, id: &str, state: &str) -> Result<()> {
        sqlx::query("UPDATE download_jobs SET state = ?, last_error = NULL, next_attempt_at = 0 WHERE id = ?")
            .bind(state).bind(id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn failed(&self, job: &DownloadJob, error: &str) -> Result<()> {
        // Keep 'submitting' on an ambiguous network failure, so reconciliation comes first.
        let delay = 30 * (1_i64 << job.attempts.clamp(0, 7));
        sqlx::query("UPDATE download_jobs SET attempts = attempts + 1, last_error = ?, next_attempt_at = ? WHERE id = ?")
            .bind(error).bind(now() + delay.min(3600)).bind(&job.id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn retry(&self, id: &str) -> Result<()> {
        let result = sqlx::query("UPDATE download_jobs SET state = 'queued', attempts = 0, next_attempt_at = 0, last_error = NULL WHERE id = ? AND state IN ('failed', 'missing')")
            .bind(id).execute(&self.pool).await?;
        if result.rows_affected() == 0 {
            return Err(Error::Invalid(
                "only failed or missing jobs can be retried".into(),
            ));
        }
        Ok(())
    }
}
