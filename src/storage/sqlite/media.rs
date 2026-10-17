// SPDX-License-Identifier: GPL-2.0-only
use super::{now, Store};
use crate::{
    domain::DownloadJob,
    error::{Error, Result},
    media::*,
};

impl Store {
    pub async fn destinations(&self) -> Result<Vec<Destination>> {
        Ok(sqlx::query_as("SELECT d.*, COALESCE(s.destination_id = d.id, 0) AS is_active FROM media_destinations d CROSS JOIN destination_selection s ORDER BY d.rowid")
            .fetch_all(&self.pool).await?)
    }

    pub async fn destination(&self, id: &str) -> Result<Destination> {
        self.destinations()
            .await?
            .into_iter()
            .find(|d| d.id == id)
            .ok_or(Error::NotFound)
    }

    pub async fn create_destination(&self, mut input: DestinationInput) -> Result<Destination> {
        input.validate()?;
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO media_destinations(id, name, kind, root_path) VALUES (?, ?, 'local', ?)",
        )
        .bind(&id)
        .bind(input.name)
        .bind(input.root_path)
        .execute(&self.pool)
        .await?;
        self.destination(&id).await
    }

    pub async fn active_destination_id(&self) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT destination_id FROM destination_selection WHERE singleton = 1",
        )
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn activate_destination(&self, id: &str) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM media_destinations WHERE id = ?)")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
        if !exists {
            return Err(Error::NotFound);
        }
        let mapped: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM destination_mappings m JOIN downloader_selection s ON s.downloader_id = m.downloader_id WHERE m.destination_id = ?)")
            .bind(id).fetch_one(&mut *tx).await?;
        if !mapped {
            return Err(Error::Invalid(
                "select a downloader and configure its mapping before activating this destination"
                    .into(),
            ));
        }
        sqlx::query("UPDATE destination_selection SET destination_id = ? WHERE singleton = 1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn mappings(&self, id: &str) -> Result<Vec<Mapping>> {
        self.destination(id).await?;
        Ok(sqlx::query_as("SELECT * FROM destination_mappings WHERE destination_id = ? ORDER BY version DESC, downloader_id")
            .bind(id).fetch_all(&self.pool).await?)
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
        let version: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(version), 0) + 1 FROM destination_mappings WHERE destination_id = ? AND downloader_id = ?")
            .bind(destination_id).bind(&input.downloader_id).fetch_one(&mut *tx).await?;
        sqlx::query("INSERT INTO destination_mappings(id, destination_id, downloader_id, version, download_root) VALUES (?, ?, ?, ?, ?)")
            .bind(&id).bind(destination_id).bind(&input.downloader_id).bind(version).bind(&root).execute(&mut *tx).await?;
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
        sqlx::query_as("SELECT * FROM download_jobs WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error::NotFound)
    }

    pub async fn file_report(&self, id: &str) -> Result<FileReport> {
        let job = self.job(id).await?;
        let files = sqlx::query_as("SELECT relative_path, expected_size, observed_size, status, error FROM media_files WHERE job_id = ? ORDER BY relative_path")
            .bind(id).fetch_all(&self.pool).await?;
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
            sqlx::query("INSERT INTO media_files(job_id, relative_path, expected_size) VALUES (?, ?, ?) ON CONFLICT(job_id, relative_path) DO NOTHING")
                .bind(id).bind(&file.relative_path).bind(file.expected_size).execute(&mut *tx).await?;
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
            sqlx::query("UPDATE media_files SET observed_size = ?, status = ?, error = ? WHERE job_id = ? AND relative_path = ?")
                .bind(file.observed_size).bind(&file.status).bind(&file.error).bind(id).bind(&file.relative_path).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE download_jobs SET file_status = ?, file_error = ?, file_checked_at = ?, file_last_success_at = CASE WHEN ? IN ('present', 'absent') THEN ? ELSE file_last_success_at END WHERE id = ?")
            .bind(status).bind(error).bind(checked_at).bind(status).bind(checked_at).bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }
}
