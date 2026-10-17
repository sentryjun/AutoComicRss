// SPDX-License-Identifier: GPL-2.0-only
use crate::error::{Error, Result};
use std::{env, time::Duration};

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub port: u16,
    pub poll_interval: Duration,
    pub qbit: Option<QbitConfig>,
    pub mikan_base_url: String,
    pub website: WebsiteConfig,
}

#[derive(Clone, Default)]
pub struct WebsiteConfig {
    pub proxy_url: Option<String>,
    pub direct_hosts: String,
}

// Deliberately omit Debug: these fields can contain credentials.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QbitConfig {
    pub url: String,
    pub username: String,
    pub password: String,
    pub save_path: String,
}

impl QbitConfig {
    pub fn validate(&mut self) -> Result<()> {
        self.url = self.url.trim().trim_end_matches('/').to_owned();
        crate::domain::validate_http_url(&self.url)?;
        let url = reqwest::Url::parse(&self.url).expect("validated URL");
        if url.query().is_some() || url.fragment().is_some() {
            return Err(Error::Invalid(
                "downloader URL cannot contain query parameters or a fragment".into(),
            ));
        }
        self.url = url.to_string().trim_end_matches('/').to_owned();
        if self.username.len() > 1024
            || self.password.len() > 4096
            || self.save_path.trim().is_empty()
            || self.save_path.len() > 4096
            || self.save_path.chars().any(char::is_control)
        {
            return Err(Error::Invalid(
                "invalid downloader credentials or save path".into(),
            ));
        }
        Ok(())
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let port = env::var("APP_PORT")
            .unwrap_or_else(|_| "8080".into())
            .parse()
            .map_err(|_| Error::Invalid("APP_PORT must be a valid port".into()))?;
        let seconds: u64 = env::var("POLL_INTERVAL_SECONDS")
            .unwrap_or_else(|_| "300".into())
            .parse()
            .map_err(|_| Error::Invalid("POLL_INTERVAL_SECONDS must be an integer".into()))?;
        if seconds < 10 {
            return Err(Error::Invalid(
                "POLL_INTERVAL_SECONDS must be at least 10".into(),
            ));
        }
        let qbit = match env::var("QBIT_URL") {
            Ok(url) if !url.is_empty() => {
                crate::domain::validate_http_url(&url)?;
                let base = reqwest::Url::parse(&url).expect("validated URL");
                if base.query().is_some() || base.fragment().is_some() {
                    return Err(Error::Invalid(
                        "QBIT_URL cannot contain query parameters or a fragment".into(),
                    ));
                }
                Some(QbitConfig {
                    url: url.trim_end_matches('/').into(),
                    username: env::var("QBIT_USERNAME").unwrap_or_default(),
                    password: env::var("QBIT_PASSWORD").unwrap_or_default(),
                    save_path: env::var("QBIT_SAVE_PATH").unwrap_or_else(|_| "/downloads".into()),
                })
            }
            _ => None,
        };
        Ok(Self {
            database_url: env::var("DATABASE_URL")
                .unwrap_or_else(|_| "sqlite://data/ani.db".into()),
            port,
            poll_interval: Duration::from_secs(seconds),
            qbit,
            mikan_base_url: env::var("MIKAN_BASE_URL")
                .unwrap_or_else(|_| "https://mikanani.me".into()),
            website: WebsiteConfig {
                proxy_url: env::var("WEBSITE_PROXY_URL")
                    .ok()
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty()),
                direct_hosts: env::var("WEBSITE_DIRECT_HOSTS").unwrap_or_default(),
            },
        })
    }
}
