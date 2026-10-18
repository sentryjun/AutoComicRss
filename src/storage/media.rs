// SPDX-License-Identifier: GPL-2.0-only
use super::{backend::db_query, operations::now, Store};
use crate::{
    domain::DownloadJob,
    error::{Error, Result},
    media::*,
};

impl Store {
    pub async fn destinations(&self) -> Result<Vec<Destination>> {
        let rows: Vec<(String, String, String, String, bool, Option<String>)> = db_query!(self.pool, query_as, "SELECT d.id, d.name, d.kind, d.root_path, COALESCE(s.destination_id = d.id, 0), d.smb_config FROM media_destinations d CROSS JOIN destination_selection s ORDER BY d.rowid"; []; fetch_all)?;
        rows.into_iter()
            .map(|(id, name, kind, root_path, is_active, config)| {
                let smb = config
                    .map(|s| serde_json::from_str::<SmbConfig>(&s).map(|c| c.view()))
                    .transpose()?;
                Ok(Destination {
                    id,
                    name,
                    kind,
                    root_path,
                    is_active,
                    smb,
                })
            })
            .collect()
    }

    pub async fn destination(&self, id: &str) -> Result<Destination> {
        self.destinations()
            .await?
            .into_iter()
            .find(|d| d.id == id)
            .ok_or(Error::NotFound)
    }

    pub(crate) async fn media_access(&self, id: &str) -> Result<MediaAccess> {
        let (kind, root, config): (String, String, Option<String>) = db_query!(self.pool, query_as, "SELECT kind, root_path, smb_config FROM media_destinations WHERE id = ?"; [id]; fetch_optional)?
        .ok_or(Error::NotFound)?;
        match kind.as_str() {
            "local" => Ok(MediaAccess::Local(root)),
            "smb" => {
                let config =
                    config.ok_or_else(|| Error::Invalid("SMB configuration is missing".into()))?;
                Ok(MediaAccess::Smb {
                    root,
                    config: serde_json::from_str(&config)?,
                })
            }
            _ => Err(Error::Invalid("unsupported media destination".into())),
        }
    }

    pub async fn create_destination(&self, mut input: DestinationInput) -> Result<Destination> {
        input.validate()?;
        let id = uuid::Uuid::new_v4().to_string();
        let kind = match input.kind {
            DestinationKind::Local => "local",
            DestinationKind::Smb => "smb",
        };
        let config = input.smb.map(|c| serde_json::to_string(&c)).transpose()?;
        db_query!(self.pool, query, "INSERT INTO media_destinations(id, name, kind, root_path, smb_config) VALUES (?, ?, ?, ?, ?)"; [&id, input.name, kind, input.root_path, config]; execute)?;
        self.destination(&id).await
    }

    pub async fn active_destination_id(&self) -> Result<Option<String>> {
        Ok(
            db_query!(self.pool, query_scalar, "SELECT destination_id FROM destination_selection WHERE singleton = 1"; []; fetch_one)?,
        )
    }

    pub async fn activate_destination(&self, id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let exists: bool = db_query!(tx, query_scalar, "SELECT EXISTS(SELECT 1 FROM media_destinations WHERE id = ?)"; [id]; fetch_one)?;
        if !exists {
            return Err(Error::NotFound);
        }
        let mapped: bool = db_query!(tx, query_scalar, "SELECT EXISTS(SELECT 1 FROM destination_mappings m JOIN downloader_selection s ON s.downloader_id = m.downloader_id WHERE m.destination_id = ?)"; [id]; fetch_one)?;
        if !mapped {
            return Err(Error::Invalid(
                "select a downloader and configure its mapping before activating this destination"
                    .into(),
            ));
        }
        db_query!(tx, query, "UPDATE destination_selection SET destination_id = ? WHERE singleton = 1"; [id]; execute)?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn mappings(&self, id: &str) -> Result<Vec<Mapping>> {
        self.destination(id).await?;
        Ok(
            db_query!(self.pool, query_as, "SELECT * FROM destination_mappings WHERE destination_id = ? ORDER BY version DESC, downloader_id"; [id]; fetch_all)?,
        )
    }

    pub async fn create_mapping(
        &self,
        destination_id: &str,
        input: MappingInput,
    ) -> Result<Mapping> {
        self.destination(destination_id).await?;
        self.downloader_config(&input.downloader_id).await?;
        let root = normalize_download_root(&input.download_root)?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut tx = self.pool.begin().await?;
        let version: i64 = db_query!(tx, query_scalar, "SELECT COALESCE(MAX(version), 0) + 1 FROM destination_mappings WHERE destination_id = ? AND downloader_id = ?"; [destination_id, &input.downloader_id]; fetch_one)?;
        db_query!(tx, query, "INSERT INTO destination_mappings(id, destination_id, downloader_id, version, download_root) VALUES (?, ?, ?, ?, ?)"; [&id, destination_id, &input.downloader_id, version, &root]; execute)?;
        tx.commit().await?;
        Ok(Mapping {
            id,
            destination_id: destination_id.into(),
            downloader_id: input.downloader_id,
            version,
            download_root: root,
        })
    }

    pub async fn job(&self, id: &str) -> Result<DownloadJob> {
        db_query!(self.pool, query_as, "SELECT * FROM download_jobs WHERE id = ?"; [id]; fetch_optional)?
            .ok_or(Error::NotFound)
    }

    pub async fn file_report(&self, id: &str) -> Result<FileReport> {
        let job = self.job(id).await?;
        let files = db_query!(self.pool, query_as, "SELECT relative_path, expected_size, observed_size, status, error FROM media_files WHERE job_id = ? ORDER BY relative_path", "SELECT relative_path, expected_size, observed_size, status, error FROM media_files WHERE job_id = ? ORDER BY BINARY relative_path"; [id]; fetch_all)?;
        Ok(FileReport {
            job_id: id.into(),
            destination_id: job.destination_id,
            status: job.file_status,
            checked_at: job.file_checked_at,
            last_success_at: job.file_last_success_at,
            error: job.file_error,
            files,
        })
    }

    pub(crate) async fn save_manifest(&self, id: &str, files: &[MediaFile]) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        for file in files {
            validate_relative(&file.relative_path, false)?;
            super::ignore_duplicate(
                db_query!(tx, query, "INSERT INTO media_files(job_id, relative_path, expected_size) VALUES (?, ?, ?)"; [id, &file.relative_path, file.expected_size]; execute),
            )?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub(crate) async fn save_file_check(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
        files: &[MediaFile],
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let checked_at = now();
        for file in files {
            db_query!(tx, query, "UPDATE media_files SET observed_size = ?, status = ?, error = ? WHERE job_id = ? AND relative_path = ?", "UPDATE media_files SET observed_size = ?, status = ?, error = ? WHERE job_id = ? AND path_digest = UNHEX(SHA2(?,256))"; [file.observed_size, &file.status, &file.error, id, &file.relative_path]; execute)?;
        }
        db_query!(tx, query, "UPDATE download_jobs SET file_status = ?, file_error = ?, file_checked_at = ?, file_last_success_at = CASE WHEN ? IN ('present', 'absent') THEN ? ELSE file_last_success_at END WHERE id = ?"; [status, error, checked_at, status, checked_at, id]; execute)?;
        tx.commit().await?;
        Ok(())
    }
}
