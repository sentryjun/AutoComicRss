// SPDX-License-Identifier: GPL-2.0-only
use super::Service;
use crate::{
    domain::DownloadJob,
    downloaders::{torrent, Downloader, RemoteState},
    error::{Error, Result},
    media,
};
use std::collections::HashMap;

impl Service {
    pub(super) async fn sync_downloads(&self) -> Result<Vec<String>> {
        let jobs = self.store.active_jobs().await?;
        let mut groups: std::collections::BTreeMap<String, Vec<DownloadJob>> = Default::default();
        for job in jobs {
            if let Some(id) = &job.downloader_id {
                groups.entry(id.clone()).or_default().push(job);
            }
        }
        let mut errors = Vec::new();
        for (id, jobs) in groups {
            let result = async {
                let (kind, connection) = self.store.downloader_config(&id).await?;
                let mut downloader = Downloader::new(kind, connection)?;
                self.sync_instance(&mut downloader, jobs).await
            }
            .await;
            if let Err(error) = result {
                if matches!(error, Error::Database(_)) {
                    return Err(error);
                }
                errors.push(format!("downloader {id}: {error}"));
            }
        }
        Ok(errors)
    }

    async fn sync_instance(
        &self,
        downloader: &mut Downloader,
        jobs: Vec<DownloadJob>,
    ) -> Result<()> {
        downloader.connect().await?;
        // Never submit when querying the remote state fails.
        let remote: HashMap<_, _> = downloader
            .tasks()
            .await?
            .into_iter()
            .map(|torrent| (torrent.hash.to_ascii_lowercase(), torrent))
            .collect();
        let mut added: HashMap<String, Option<String>> = HashMap::new();
        for mut job in jobs {
            let sub = self.store.subscription(&job.subscription_id).await;
            let enabled = match sub {
                Ok(sub) => sub.input.enabled,
                Err(Error::NotFound) => false,
                Err(error) => return Err(error),
            };
            let result: Result<()> = async {
                if job.info_hash.is_none() {
                    if !enabled { return Ok(()); }
                    let prepared = torrent::prepare(&self.client, &job.download_url).await?;
                    self.store.prepare(&job.id, &prepared.hash, prepared.bytes.as_deref()).await?;
                    job.info_hash = Some(prepared.hash);
                }
                let hash = job.info_hash.as_ref().expect("prepared hash");
                if let Some(torrent) = remote.get(hash) {
                    if job.download_save_path.is_some() && !job.download_save_path.as_deref().zip(torrent.save_path.as_deref())
                        .is_some_and(|(expected, actual)| media::same_download_root(expected, actual)) {
                        return Err(Error::Invalid("remote save path differs from the job snapshot; refusing to move or reuse this torrent".into()));
                    }
                    if matches!(torrent.state, RemoteState::Failed) {
                        return Err(Error::Invalid("downloader reports a local error or missing files; inspect the downloader before retrying".into()));
                    }
                    self.store.state(&job.id, torrent.state.as_str()).await?;
                } else if let Some(path) = added.get(hash) {
                    if job.download_save_path.is_some() && path != &job.download_save_path {
                        return Err(Error::Invalid("this torrent was submitted to a different path; refusing to reuse it".into()));
                    }
                    self.store.state(&job.id, "submitted").await?;
                } else if matches!(job.state.as_str(), "submitted" | "downloading") {
                    // Respect removal in the original downloader; recreating a task needs an explicit retry.
                    self.store.state(&job.id, "missing").await?;
                } else if enabled {
                    let bytes = self.store.torrent_bytes(&job.id).await?;
                    // Commit the intent before the external side effect.
                    self.store.state(&job.id, "submitting").await?;
                    downloader.submit(hash, &job.download_url, bytes, job.download_save_path.as_deref()).await?;
                    added.insert(hash.clone(), job.download_save_path.clone());
                    self.store.state(&job.id, "submitted").await?;
                }
                Ok(())
            }.await;
            if let Err(error) = result {
                if matches!(error, Error::Database(_)) {
                    return Err(error);
                }
                if matches!(error, Error::Invalid(_)) {
                    self.store.state(&job.id, "failed").await?;
                }
                self.store.failed(&job, &error.to_string()).await?;
                tracing::warn!(job_id = %job.id, error = %error, "download job failed");
            }
        }
        Ok(())
    }
}
