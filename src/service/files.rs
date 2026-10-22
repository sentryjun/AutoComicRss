// SPDX-License-Identifier: GPL-2.0-only
use super::Service;
use crate::{
    domain::DownloadJob,
    downloaders::Downloader,
    error::{Error, Result},
    media::{self, FileReport, MediaFile},
};
use std::collections::HashSet;

impl Service {
    async fn discover_files(&self, job: &DownloadJob) -> Result<Vec<MediaFile>> {
        let hash = job
            .info_hash
            .as_ref()
            .ok_or_else(|| Error::Invalid("torrent metadata is not available yet".into()))?;
        let owner = job
            .downloader_id
            .as_ref()
            .ok_or_else(|| Error::Invalid("downloader ownership is unknown".into()))?;
        let (kind, connection) = self.store.downloader_config(owner).await?;
        let mut downloader = Downloader::new(kind, connection)?;
        downloader.connect().await?;
        let torrent = downloader
            .tasks()
            .await?
            .into_iter()
            .find(|torrent| torrent.hash.eq_ignore_ascii_case(hash))
            .ok_or_else(|| {
                Error::Upstream("torrent is not available in its original downloader".into())
            })?;
        if !job
            .download_save_path
            .as_deref()
            .zip(torrent.save_path.as_deref())
            .is_some_and(|(expected, actual)| media::same_download_root(expected, actual))
        {
            return Err(Error::Upstream(
                "remote save path does not match the frozen destination mapping".into(),
            ));
        }
        let mut files = Vec::new();
        let mut paths = HashSet::new();
        for file in downloader
            .files(hash)
            .await?
            .into_iter()
            .filter(|file| file.wanted)
        {
            media::validate_relative(&file.name, false).map_err(|_| {
                Error::Upstream("downloader returned an unsafe relative file path".into())
            })?;
            if !paths.insert(file.name.clone()) {
                return Err(Error::Upstream(
                    "duplicate path in downloader file list".into(),
                ));
            }
            files.push(MediaFile {
                relative_path: file.name,
                expected_size: file.size,
                observed_size: None,
                status: "unchecked".into(),
                error: None,
            });
        }
        Ok(files)
    }

    pub async fn check_job_files(&self, id: &str) -> Result<FileReport> {
        // Serialize manifest discovery/checks with task synchronization and configuration changes.
        let _guard = self.gate.try_lock().map_err(|_| Error::Busy)?;
        let job = self.store.job(id).await?;
        let Some(destination) = &job.destination_id else {
            self.store
                .save_file_check(
                    id,
                    "unchecked",
                    Some("job has no recorded media destination"),
                    &[],
                )
                .await?;
            return self.store.file_report(id).await;
        };
        let access = self.store.media_access(destination).await?;
        let mut files = self.store.file_report(id).await?.files;
        if files.is_empty() {
            if job.info_hash.is_none() {
                self.store
                    .save_file_check(
                        id,
                        "unchecked",
                        Some("torrent metadata is not available yet"),
                        &[],
                    )
                    .await?;
                return self.store.file_report(id).await;
            }
            match self.discover_files(&job).await {
                Ok(manifest) => files = manifest,
                Err(error @ Error::Database(_)) => return Err(error),
                Err(error) => {
                    self.store
                        .save_file_check(id, "unavailable", Some(&error.to_string()), &[])
                        .await?;
                    return self.store.file_report(id).await;
                }
            }
            if files.is_empty() {
                self.store
                    .save_file_check(
                        id,
                        "unchecked",
                        Some("no selected files or torrent metadata is still pending"),
                        &[],
                    )
                    .await?;
                return self.store.file_report(id).await;
            }
            self.store.save_manifest(id, &files).await?;
        }
        let files = access.check(files).await?;
        let status = if files.iter().any(|f| f.status == "unavailable") {
            "unavailable"
        } else if files.iter().any(|f| f.status == "absent") {
            "absent"
        } else {
            "present"
        };
        let error = files.iter().find_map(|file| file.error.as_deref());
        self.store
            .save_file_check(id, status, error, &files)
            .await?;
        self.store.file_report(id).await
    }
}
