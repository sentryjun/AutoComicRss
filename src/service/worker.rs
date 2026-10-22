// SPDX-License-Identifier: GPL-2.0-only
use super::Service;
use crate::error::{Error, Result};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Default, Serialize)]
pub struct CycleReport {
    pub scanned: usize,
    pub queued: usize,
    pub errors: Vec<String>,
    pub downloader_configured: bool,
}

impl Service {
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
