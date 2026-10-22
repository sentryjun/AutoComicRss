// SPDX-License-Identifier: GPL-2.0-only
use super::{backend::db_query, now, Store};
use crate::{
    domain::{Decision, Release, Subscription, SubscriptionInput},
    error::{Error, Result},
};

impl Store {
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
}
