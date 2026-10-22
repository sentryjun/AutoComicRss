// SPDX-License-Identifier: GPL-2.0-only
//! Application workflows coordinating sources, persistence, downloaders and media.
mod downloads;
mod files;
mod subscriptions;
mod worker;

use crate::{config::Config, error::Result, storage::Store};
use tokio::sync::Mutex;
pub use worker::{worker, CycleReport};

pub struct Service {
    pub store: Store,
    pub gate: Mutex<()>,
    pub bangumi: crate::sources::bangumi::Bangumi,
    pub mikan: crate::sources::mikan::Mikan,
    pub scraper: crate::scraping::Scraper,
    client: reqwest::Client,
}

impl Service {
    pub async fn new(store: Store, config: &Config) -> Result<Self> {
        let client = crate::network::website_client(&config.website)?;
        let mikan = crate::sources::mikan::Mikan::new(client.clone(), &config.mikan_base_url)?;
        store.bootstrap_downloader(config.qbit.as_ref()).await?;
        Ok(Self {
            store,
            gate: Mutex::new(()),
            bangumi: crate::sources::bangumi::Bangumi::new(
                client.clone(),
                &config.bangumi_base_url,
            )?,
            mikan,
            scraper: crate::scraping::Scraper::new()?,
            client,
        })
    }

    pub async fn downloader_configured(&self) -> Result<bool> {
        Ok(self.store.active_downloader_id().await?.is_some())
    }
}
