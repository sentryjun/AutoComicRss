// SPDX-License-Identifier: GPL-2.0-only
use super::{backend::db_query, Store};
use crate::{
    domain::{DownloaderInput, DownloaderKind, DownloaderView},
    error::{Error, Result},
};

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
}
