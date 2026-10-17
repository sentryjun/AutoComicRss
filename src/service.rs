// SPDX-License-Identifier: GPL-2.0-only
use crate::{
    config::Config,
    domain::*,
    downloaders::{qbittorrent::Qbit, torrent},
    error::{Error, Result},
    media::{self, FileReport, MediaFile},
    sources::feeds,
    storage::Store,
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::{watch, Mutex};

pub struct Service {
    pub store: Store,
    pub gate: Mutex<()>,
    pub mikan: crate::sources::mikan::Mikan,
    client: reqwest::Client,
}

#[derive(Default, Serialize)]
pub struct CycleReport {
    pub scanned: usize,
    pub queued: usize,
    pub errors: Vec<String>,
    pub downloader_configured: bool,
}

impl Service {
    pub async fn new(store: Store, config: &Config) -> Result<Self> {
        let client = crate::network::website_client(&config.website)?;
        let mikan = crate::sources::mikan::Mikan::new(client.clone(), &config.mikan_base_url)?;
        store.bootstrap_downloader(config.qbit.as_ref()).await?;
        Ok(Self {
            store,
            gate: Mutex::new(()),
            mikan,
            client,
        })
    }

    pub async fn downloader_configured(&self) -> Result<bool> {
        Ok(self.store.active_downloader_id().await?.is_some())
    }

    pub async fn preview(&self, mut input: SubscriptionInput) -> Result<Vec<Release>> {
        input.validate()?;
        feeds::fetch(&self.client, &input).await
    }

    pub async fn preview_subscription(&self, id: &str) -> Result<Vec<Release>> {
        let sub = self.store.subscription(id).await?;
        let mut releases = self.preview(sub.input.clone()).await?;
        let mut episodes = HashSet::new();
        // Feed order determines the candidate selected in v0.1; priority rules come later.
        for release in &mut releases {
            if release.decision == Decision::Ready {
                let duplicate = self
                    .store
                    .already_queued(id, sub.input.season, release)
                    .await?;
                if duplicate || !episodes.insert(release.episode) {
                    release.decision = Decision::AlreadyQueued;
                    release.reason = if duplicate {
                        "resource or episode already has a task"
                    } else {
                        "an earlier feed entry already represents this episode"
                    }
                    .into();
                }
            }
        }
        Ok(releases)
    }

    async fn scan(&self, sub: &Subscription) -> Result<usize> {
        let result = async {
            let releases = self.preview_subscription(&sub.id).await?;
            self.store.record_scan(sub, &releases).await
        }
        .await;
        self.store
            .checked(
                &sub.id,
                result.as_ref().err().map(ToString::to_string).as_deref(),
            )
            .await?;
        result
    }

    pub async fn scan_subscription(&self, id: &str) -> Result<usize> {
        let _guard = self.gate.try_lock().map_err(|_| Error::Busy)?;
        let sub = self.store.subscription(id).await?;
        if !sub.input.enabled {
            return Err(Error::Invalid(
                "enable the subscription before scanning; preview is available while disabled"
                    .into(),
            ));
        }
        self.scan(&sub).await
    }

    pub async fn cycle(&self) -> Result<CycleReport> {
        let _guard = self.gate.try_lock().map_err(|_| Error::Busy)?;
        let mut report = CycleReport {
            downloader_configured: self.downloader_configured().await?,
            ..Default::default()
        };
        let legacy = self.store.legacy_unassigned_count().await?;
        if legacy > 0 {
            report.errors.push(format!("{legacy} legacy jobs have no recorded downloader; restore their original instance and explicitly claim them"));
        }
        for sub in self.store.subscriptions().await? {
            if !sub.input.enabled {
                continue;
            }
            match self.scan(&sub).await {
                Ok(count) => {
                    report.scanned += 1;
                    report.queued += count;
                }
                Err(error) => report
                    .errors
                    .push(format!("subscription {}: {error}", sub.id)),
            }
        }
        match self.sync_downloads().await {
            Ok(errors) => report.errors.extend(errors),
            Err(error) => report.errors.push(error.to_string()),
        }
        Ok(report)
    }

    async fn sync_downloads(&self) -> Result<Vec<String>> {
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
                let qbit = Qbit::new(self.store.downloader_config(&id).await?)?;
                self.sync_instance(&qbit, jobs).await
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

    async fn sync_instance(&self, qbit: &Qbit, jobs: Vec<DownloadJob>) -> Result<()> {
        qbit.login().await?;
        // Never submit when querying the remote state fails.
        let remote: HashMap<_, _> = qbit
            .torrents()
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
                    if job.destination_id.is_some() && !job.download_save_path.as_deref().zip(torrent.save_path.as_deref())
                        .is_some_and(|(expected, actual)| media::same_download_root(expected, actual)) {
                        return Err(Error::Invalid("remote save path differs from the job snapshot; refusing to move or reuse this torrent".into()));
                    }
                    if matches!(torrent.state.as_str(), "error" | "missingFiles") {
                        return Err(Error::Invalid("qBittorrent reports an error or missing files; inspect the downloader before retrying".into()));
                    }
                    let state = if torrent.progress >= 1.0 { "completed" }
                        else if torrent.progress > 0.0 { "downloading" } else { "submitted" };
                    self.store.state(&job.id, state).await?;
                } else if let Some(path) = added.get(hash) {
                    if job.destination_id.is_some() && path != &job.download_save_path {
                        return Err(Error::Invalid("this torrent was submitted to a different path; refusing to reuse it".into()));
                    }
                    self.store.state(&job.id, "submitted").await?;
                } else if matches!(job.state.as_str(), "submitted" | "downloading") {
                    // Respect removal in qBittorrent; recreating a task needs an explicit retry.
                    self.store.state(&job.id, "missing").await?;
                } else if enabled {
                    let bytes = self.store.torrent_bytes(&job.id).await?;
                    // Commit the intent before the external side effect.
                    self.store.state(&job.id, "submitting").await?;
                    qbit.add(&job.download_url, bytes, job.download_save_path.as_deref()).await?;
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

    async fn discover_files(&self, job: &DownloadJob) -> Result<Vec<MediaFile>> {
        let hash = job
            .info_hash
            .as_ref()
            .ok_or_else(|| Error::Invalid("torrent metadata is not available yet".into()))?;
        let owner = job
            .downloader_id
            .as_ref()
            .ok_or_else(|| Error::Invalid("downloader ownership is unknown".into()))?;
        let qbit = Qbit::new(self.store.downloader_config(owner).await?)?;
        qbit.login().await?;
        let torrent = qbit
            .torrents()
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
        for file in qbit
            .files(hash)
            .await?
            .into_iter()
            .filter(|file| file.priority != 0)
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
        let root = self.store.destination(destination).await?.root_path;
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
        let files = tokio::task::spawn_blocking(move || media::check_local(&root, files))
            .await
            .map_err(|_| Error::Upstream("local file check failed".into()))?;
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

pub async fn worker(
    service: Arc<Service>,
    interval: std::time::Duration,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut timer = tokio::time::interval(interval);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            _ = timer.tick() => {
                // Cancellation can interrupt network I/O; persisted submission intent allows recovery.
                tokio::select! {
                    _ = shutdown.changed() => break,
                    result = service.cycle() => match result {
                        Ok(report) => for error in report.errors { tracing::warn!(%error, "background cycle issue"); },
                        Err(Error::Busy) => {},
                        Err(error) => tracing::error!(%error, "background cycle failed"),
                    }
                }
            }
        }
    }
}
