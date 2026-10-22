// SPDX-License-Identifier: GPL-2.0-only
use super::Service;
use crate::{
    domain::{Decision, Release, Subscription, SubscriptionInput},
    error::{Error, Result},
    sources::feeds,
};
use std::collections::HashSet;

impl Service {
    pub async fn preview(&self, mut input: SubscriptionInput) -> Result<Vec<Release>> {
        input.validate()?;
        let settings = self.store.scraping_settings().await?;
        let mut releases = feeds::fetch_candidates(&self.client, &input).await?;
        self.scraper.apply(&settings, &input, &mut releases).await;
        Ok(releases)
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

    pub(super) async fn scan(&self, sub: &Subscription) -> Result<usize> {
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
}
