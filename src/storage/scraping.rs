// SPDX-License-Identifier: GPL-2.0-only
use super::{backend::db_query, Store};
use crate::{
    error::Result,
    scraping::{Settings, SettingsInput},
};
impl Store {
    pub async fn scraping_settings(&self) -> Result<Settings> {
        let json: Option<String> = db_query!(self.pool, query_scalar, "SELECT config FROM scraping_settings WHERE singleton = 1"; []; fetch_optional)?;
        Ok(match json {
            Some(json) => serde_json::from_str(&json)?,
            None => Settings::default(),
        })
    }
    pub async fn save_scraping_settings(&self, input: SettingsInput) -> Result<Settings> {
        let settings = self.scraping_settings().await?.updated(input)?;
        let json = serde_json::to_string(&settings)?;
        db_query!(self.pool, query, "INSERT INTO scraping_settings(singleton,config) VALUES (1,?) ON CONFLICT(singleton) DO UPDATE SET config=excluded.config", "INSERT INTO scraping_settings(singleton,config) VALUES (1,?) ON DUPLICATE KEY UPDATE config=VALUES(config)"; [&json]; execute)?;
        Ok(settings)
    }
}
