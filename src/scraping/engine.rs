// SPDX-License-Identifier: GPL-2.0-only
use super::{llm, regex, Metadata, Mode, ParsedTitle, Settings};
use crate::{
    domain::{Decision, Release, SubscriptionInput},
    error::{Error, Result},
};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};

pub struct Scraper {
    client: reqwest::Client,
    cache: Mutex<HashMap<(String, String), (Instant, Metadata)>>,
    requests: Semaphore,
}
impl Scraper {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(10))
                .build()?,
            cache: Mutex::new(HashMap::new()),
            requests: Semaphore::new(2),
        })
    }
    pub async fn parse(
        &self,
        settings: &Settings,
        title: &str,
        season: i32,
    ) -> Result<ParsedTitle> {
        if title.is_empty() || title.len() > 4096 {
            return Err(Error::Invalid(
                "resource title must contain 1–4096 bytes".into(),
            ));
        }
        let metadata = match settings.mode {
            Mode::Regex => regex::parse(title, season),
            Mode::Llm => {
                let key = (settings.revision.clone(), title.to_owned());
                if let Some((at, value)) = self.cache.lock().await.get(&key) {
                    if at.elapsed() < Duration::from_secs(3600) {
                        return Ok(ParsedTitle {
                            source: Mode::Llm,
                            metadata: value.clone(),
                        });
                    }
                }
                // Limit remote calls; cache hits remain available while the provider is busy.
                let _permit = self.requests.try_acquire().map_err(|_| Error::SourceBusy)?;
                let result = llm::parse(&self.client, settings, title).await?;
                let mut cache = self.cache.lock().await;
                cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(3600));
                if cache.len() >= 2048 {
                    cache.clear();
                }
                cache.insert(key, (Instant::now(), result.clone()));
                result
            }
        };
        Ok(ParsedTitle {
            source: settings.mode,
            metadata,
        })
    }
    pub async fn apply(
        &self,
        settings: &Settings,
        sub: &SubscriptionInput,
        releases: &mut [Release],
    ) {
        let started = Instant::now();
        let mut requests = 0;
        let mut failed = false;
        for release in releases
            .iter_mut()
            .filter(|r| r.decision == Decision::Ready)
        {
            let cached = self
                .cache
                .lock()
                .await
                .get(&(settings.revision.clone(), release.title.clone()))
                .is_some_and(|(at, _)| at.elapsed() < Duration::from_secs(3600));
            let result = if settings.mode == Mode::Llm
                && (failed
                    || (!cached && requests >= 20)
                    || started.elapsed() >= Duration::from_secs(60))
            {
                Err(Error::Upstream("LLM parsing paused after an error or request/time limit; preview again to continue".into()))
            } else {
                if settings.mode == Mode::Llm && !cached {
                    requests += 1;
                }
                let remaining = Duration::from_secs(60).saturating_sub(started.elapsed());
                if settings.mode == Mode::Llm {
                    tokio::time::timeout(
                        remaining,
                        self.parse(settings, &release.title, sub.season),
                    )
                    .await
                    .unwrap_or_else(|_| {
                        Err(Error::Upstream("LLM preview time limit reached".into()))
                    })
                } else {
                    self.parse(settings, &release.title, sub.season).await
                }
            };
            match result {
                Ok(parsed) => {
                    let episode = parsed.episode(sub.season, sub.episode_offset);
                    release.metadata = Some(parsed);
                    match episode {
                        Ok(ep) => release.episode = Some(ep),
                        Err(reason) => review(release, reason),
                    }
                }
                Err(error) => {
                    failed = true;
                    review(release, &error.to_string());
                }
            }
        }
    }
}
fn review(release: &mut Release, reason: &str) {
    release.episode = None;
    release.decision = Decision::NeedsReview;
    release.reason = reason.into();
}
