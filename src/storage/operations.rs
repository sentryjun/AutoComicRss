// SPDX-License-Identifier: GPL-2.0-only
use super::backend::{db_query, Pool};

use crate::{
    domain::*,
    error::{Error, Result},
};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct Store {
    pub pool: Pool,
}

pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

impl Store {
    pub async fn active_downloader_id(&self) -> Result<Option<String>> {
        Ok(
            db_query!(self.pool, query_scalar, "SELECT downloader_id FROM downloader_selection WHERE singleton = 1"; []; fetch_one)?,
        )
    }

    pub async fn downloaders(&self) -> Result<Vec<DownloaderView>> {
        let rows: Vec<(String, String, String, String, bool)> = db_query!(self.pool, query_as, "SELECT d.id, d.name, d.kind, d.config, COALESCE(s.downloader_id = d.id, 0) FROM downloader_instances d CROSS JOIN downloader_selection s ORDER BY d.rowid"; []; fetch_all)?;
        rows.into_iter()
            .map(|(id, name, kind, json, is_active)| {
                let config: crate::config::DownloaderConnection = serde_json::from_str(&json)?;
                Ok(DownloaderView {
                    id,
                    name,
                    kind: DownloaderKind::from_stored(&kind)?,
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
        let result = db_query!(self.pool, query, "INSERT INTO downloader_instances(id, name, kind, endpoint, config) VALUES (?, ?, ?, ?, ?)"; [&id, &input.name, input.kind.as_str(), &input.connection.url, serde_json::to_string(&input.connection)?]; execute);
        if super::ignore_duplicate(result)? == 0 {
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

    pub(crate) async fn downloader_config(
        &self,
        id: &str,
    ) -> Result<(DownloaderKind, crate::config::DownloaderConnection)> {
        let (kind, json): (String, String) =
            db_query!(self.pool, query_as, "SELECT kind, config FROM downloader_instances WHERE id = ?"; [id]; fetch_optional)?
                .ok_or(Error::NotFound)?;
        Ok((
            DownloaderKind::from_stored(&kind)?,
            serde_json::from_str(&json)?,
        ))
    }

    pub async fn activate_downloader(&self, id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let exists: bool = db_query!(tx, query_scalar, "SELECT EXISTS(SELECT 1 FROM downloader_instances WHERE id = ?)"; [id]; fetch_one)?;
        if !exists {
            return Err(Error::NotFound);
        }
        let destination: Option<String> = db_query!(tx, query_scalar, "SELECT destination_id FROM destination_selection WHERE singleton = 1"; []; fetch_one)?;
        if let Some(destination) = destination {
            let mapped: bool = db_query!(tx, query_scalar, "SELECT EXISTS(SELECT 1 FROM destination_mappings WHERE destination_id = ? AND downloader_id = ?)"; [destination, id]; fetch_one)?;
            if !mapped {
                return Err(Error::Invalid("configure a mapping to the active destination before selecting this downloader".into()));
            }
        }
        db_query!(tx, query, "UPDATE downloader_selection SET downloader_id = ? WHERE singleton = 1"; [id]; execute)?;
        // Bind only new, never-owned work. A switch never rewrites an existing owner.
        db_query!(tx, query, "UPDATE download_jobs SET downloader_id = ? WHERE downloader_id IS NULL AND legacy_unassigned = 0"; [id]; execute)?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn claim_legacy_jobs(&self, id: &str) -> Result<u64> {
        let kind: String = db_query!(self.pool, query_scalar, "SELECT kind FROM downloader_instances WHERE id = ?"; [id]; fetch_optional)?
            .ok_or(Error::NotFound)?;
        if kind != "qbittorrent" {
            return Err(Error::Invalid(
                "legacy jobs must be claimed by their original qBittorrent instance".into(),
            ));
        }
        Ok(
            db_query!(self.pool, query, "UPDATE download_jobs SET downloader_id = ?, legacy_unassigned = 0 WHERE downloader_id IS NULL AND legacy_unassigned = 1"; [id]; execute)?,
        )
    }

    pub async fn legacy_unassigned_count(&self) -> Result<i64> {
        Ok(
            db_query!(self.pool, query_scalar, "SELECT COUNT(*) FROM download_jobs WHERE legacy_unassigned = 1 AND downloader_id IS NULL"; []; fetch_one)?,
        )
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
        let count: i64 = db_query!(tx, query_scalar, "SELECT COUNT(*) FROM downloader_instances"; []; fetch_one)?;
        if count == 0 {
            let id = uuid::Uuid::new_v4().to_string();
            db_query!(tx, query, "INSERT INTO downloader_instances(id, name, kind, endpoint, config) VALUES (?, 'Initial qBittorrent', 'qbittorrent', ?, ?)"; [&id, &config.url, serde_json::to_string(&config)?]; execute)?;
            db_query!(tx, query, "UPDATE downloader_selection SET downloader_id = ? WHERE singleton = 1"; [&id]; execute)?;
            // Initial import uses the operator-supplied original QBIT_* connection.
            db_query!(tx, query, "UPDATE download_jobs SET downloader_id = ?, legacy_unassigned = 0 WHERE downloader_id IS NULL"; [&id]; execute)?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn open(url: &str) -> Result<Self> {
        Ok(Self {
            pool: Pool::open(url).await?,
        })
    }

    pub async fn subscriptions(&self) -> Result<Vec<Subscription>> {
        let rows: Vec<(String, String, Option<i64>, Option<String>)> = db_query!(self.pool, query_as, "SELECT id, spec, last_checked_at, last_error FROM subscriptions WHERE deleted = 0 ORDER BY rowid"; []; fetch_all)?;
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
        db_query!(self.pool, query, "INSERT INTO subscriptions(id, spec) VALUES (?, ?)"; [&id, serde_json::to_string(&input)?]; execute)?;
        self.subscription(&id).await
    }

    pub async fn update(&self, id: &str, mut input: SubscriptionInput) -> Result<Subscription> {
        input.validate()?;
        let result = db_query!(self.pool, query, "UPDATE subscriptions SET spec = ? WHERE id = ? AND deleted = 0"; [serde_json::to_string(&input)?, id]; execute)?;
        if result == 0 {
            return Err(Error::NotFound);
        }
        self.subscription(id).await
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let result = db_query!(self.pool, query, "UPDATE subscriptions SET deleted = 1 WHERE id = ? AND deleted = 0"; [id]; execute)?;
        if result == 0 {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    pub async fn checked(&self, id: &str, error: Option<&str>) -> Result<()> {
        db_query!(self.pool, query, "UPDATE subscriptions SET last_checked_at = ?, last_error = ? WHERE id = ?"; [now(), error, id]; execute)?;
        Ok(())
    }

    pub async fn already_queued(&self, id: &str, season: i32, release: &Release) -> Result<bool> {
        let count: i64 = db_query!(self.pool, query_scalar, "SELECT COUNT(*) FROM download_jobs WHERE subscription_id = ? AND (resource_key = ? OR (season = ? AND episode = ?))", "SELECT COUNT(*) FROM download_jobs WHERE subscription_id = ? AND (resource_digest = UNHEX(SHA2(?,256)) OR (season = ? AND episode = ?))"; [id, &release.key, season, release.episode]; fetch_one)?;
        Ok(count > 0)
    }

    pub async fn record_scan(&self, sub: &Subscription, releases: &[Release]) -> Result<usize> {
        let mut tx = self.pool.begin().await?;
        let downloader_id: Option<String> = db_query!(tx, query_scalar, "SELECT downloader_id FROM downloader_selection WHERE singleton = 1"; []; fetch_one)?;
        let destination_id: Option<String> = db_query!(tx, query_scalar, "SELECT destination_id FROM destination_selection WHERE singleton = 1"; []; fetch_one)?;
        let mut mapping_id = None;
        let mut download_save_path = None;
        if let Some(destination) = &destination_id {
            let mapping: crate::media::Mapping = db_query!(tx, query_as, "SELECT * FROM destination_mappings WHERE destination_id = ? AND downloader_id = ? ORDER BY version DESC LIMIT 1"; [destination, &downloader_id]; fetch_optional)?
                .ok_or_else(|| Error::Invalid("active destination has no mapping for the selected downloader".into()))?;
            mapping_id = Some(mapping.id);
            download_save_path = Some(mapping.download_root);
        } else if let Some(downloader) = &downloader_id {
            let config: String = db_query!(tx, query_scalar, "SELECT config FROM downloader_instances WHERE id = ?"; [downloader]; fetch_one)?;
            download_save_path = Some(
                serde_json::from_str::<crate::config::DownloaderConnection>(&config)?.save_path,
            );
        }
        let mut queued = 0;
        for release in releases {
            db_query!(tx, query, "INSERT INTO releases(subscription_id, resource_key, payload) VALUES (?, ?, ?) ON CONFLICT(subscription_id, resource_key) DO UPDATE SET payload = excluded.payload", "INSERT INTO releases(subscription_id, resource_key, payload) VALUES (?, ?, ?) AS incoming ON DUPLICATE KEY UPDATE payload = incoming.payload"; [&sub.id, &release.key, serde_json::to_string(release)?]; execute)?;
            if release.decision == Decision::Ready {
                if let (Some(episode), Some(url)) = (release.episode, &release.download_url) {
                    let result = db_query!(tx, query, "INSERT INTO download_jobs(id, subscription_id, resource_key, title, season, episode, download_url, downloader_id, destination_id, mapping_id, download_save_path) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"; [uuid::Uuid::new_v4().to_string(), &sub.id, &release.key, &release.title, sub.input.season, episode, url, &downloader_id, &destination_id, &mapping_id, &download_save_path]; execute);
                    queued += super::ignore_duplicate(result)? as usize;
                }
            }
        }
        tx.commit().await?;
        Ok(queued)
    }

    pub async fn releases(&self, id: &str) -> Result<Vec<Release>> {
        self.subscription(id).await?;
        let rows: Vec<String> = db_query!(self.pool, query_scalar, "SELECT payload FROM releases WHERE subscription_id = ? ORDER BY rowid DESC LIMIT 500"; [id]; fetch_all)?;
        rows.into_iter()
            .map(|s| serde_json::from_str(&s).map_err(Error::from))
            .collect()
    }

    pub async fn jobs(&self) -> Result<Vec<DownloadJob>> {
        Ok(
            db_query!(self.pool, query_as, "SELECT * FROM download_jobs ORDER BY rowid DESC LIMIT 500"; []; fetch_all)?,
        )
    }

    pub async fn active_jobs(&self) -> Result<Vec<DownloadJob>> {
        Ok(
            db_query!(self.pool, query_as, "SELECT * FROM download_jobs WHERE state NOT IN ('completed', 'missing', 'failed') AND next_attempt_at <= ? ORDER BY rowid"; [now()]; fetch_all)?,
        )
    }

    pub async fn prepare(&self, id: &str, hash: &str, bytes: Option<&[u8]>) -> Result<()> {
        db_query!(self.pool, query, "UPDATE download_jobs SET info_hash = ?, torrent_bytes = ? WHERE id = ?"; [hash, bytes, id]; execute)?;
        Ok(())
    }

    pub async fn torrent_bytes(&self, id: &str) -> Result<Option<Vec<u8>>> {
        Ok(
            db_query!(self.pool, query_scalar, "SELECT torrent_bytes FROM download_jobs WHERE id = ?"; [id]; fetch_one)?,
        )
    }

    pub async fn state(&self, id: &str, state: &str) -> Result<()> {
        db_query!(self.pool, query, "UPDATE download_jobs SET state = ?, last_error = NULL, next_attempt_at = 0 WHERE id = ?"; [state, id]; execute)?;
        Ok(())
    }

    pub async fn failed(&self, job: &DownloadJob, error: &str) -> Result<()> {
        // Keep 'submitting' on an ambiguous network failure, so reconciliation comes first.
        let delay = 30 * (1_i64 << job.attempts.clamp(0, 7));
        db_query!(self.pool, query, "UPDATE download_jobs SET attempts = attempts + 1, last_error = ?, next_attempt_at = ? WHERE id = ?"; [error, now() + delay.min(3600), &job.id]; execute)?;
        Ok(())
    }

    pub async fn retry(&self, id: &str) -> Result<()> {
        let result = db_query!(self.pool, query, "UPDATE download_jobs SET state = 'queued', attempts = 0, next_attempt_at = 0, last_error = NULL WHERE id = ? AND state IN ('failed', 'missing')"; [id]; execute)?;
        if result == 0 {
            return Err(Error::Invalid(
                "only failed or missing jobs can be retried".into(),
            ));
        }
        Ok(())
    }
}
