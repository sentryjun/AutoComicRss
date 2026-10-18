// SPDX-License-Identifier: GPL-2.0-only
//! Subscription, release and download job models.
pub mod episode;

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionInput {
    pub title: String,
    pub rss_url: String,
    #[serde(default = "one")]
    pub season: i32,
    #[serde(default)]
    pub episode_offset: i32,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    // Opt in after previewing: the first scan considers all entries in the feed.
    #[serde(default)]
    pub enabled: bool,
}

fn one() -> i32 {
    1
}

impl SubscriptionInput {
    pub fn validate(&mut self) -> Result<()> {
        self.title = self.title.trim().to_owned();
        self.rss_url = self.rss_url.trim().to_owned();
        if self.title.is_empty() || self.title.chars().count() > 200 {
            return Err(Error::Invalid("title must contain 1–200 characters".into()));
        }
        validate_http_url(&self.rss_url)?;
        if !(1..=999).contains(&self.season) || self.episode_offset.abs_diff(0) > 10000 {
            return Err(Error::Invalid(
                "season or episode_offset is out of range".into(),
            ));
        }
        for rules in [&mut self.include, &mut self.exclude] {
            if rules.len() > 32 {
                return Err(Error::Invalid("at most 32 rules per list".into()));
            }
            for rule in rules {
                *rule = rule.trim().to_owned();
                if rule.is_empty() || rule.len() > 200 {
                    return Err(Error::Invalid(
                        "filter keywords must contain 1–200 bytes".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

pub fn validate_http_url(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value).map_err(|_| Error::Invalid("invalid URL".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || value.len() > 8192
    {
        return Err(Error::Invalid(
            "expected an HTTP(S) URL without embedded credentials".into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    #[serde(flatten)]
    pub input: SubscriptionInput,
    pub last_checked_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Ready,
    Filtered,
    NeedsReview,
    AlreadyQueued,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    pub key: String,
    pub title: String,
    pub download_url: Option<String>,
    pub episode: Option<i32>,
    pub decision: Decision,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct DownloadJob {
    pub id: String,
    pub subscription_id: String,
    pub title: String,
    pub season: i32,
    pub episode: i32,
    pub download_url: String,
    pub info_hash: Option<String>,
    pub state: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub next_attempt_at: i64,
    pub downloader_id: Option<String>,
    pub legacy_unassigned: bool,
    pub destination_id: Option<String>,
    pub mapping_id: Option<String>,
    pub download_save_path: Option<String>,
    pub file_status: String,
    pub file_checked_at: Option<i64>,
    pub file_last_success_at: Option<i64>,
    pub file_error: Option<String>,
}

#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloaderInput {
    pub name: String,
    pub kind: DownloaderKind,
    pub connection: crate::config::DownloaderConnection,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloaderKind {
    Qbittorrent,
    Transmission,
}

impl DownloaderKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Qbittorrent => "qbittorrent",
            Self::Transmission => "transmission",
        }
    }
    pub(crate) fn from_stored(value: &str) -> Result<Self> {
        match value {
            "qbittorrent" => Ok(Self::Qbittorrent),
            "transmission" => Ok(Self::Transmission),
            _ => Err(Error::Invalid("unsupported stored downloader kind".into())),
        }
    }
}

impl DownloaderInput {
    pub fn validate(&mut self) -> Result<()> {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty() || self.name.chars().count() > 100 {
            return Err(Error::Invalid(
                "downloader name must contain 1–100 characters".into(),
            ));
        }
        self.connection.validate()
    }
}

#[derive(serde::Serialize)]
pub struct DownloaderView {
    pub id: String,
    pub name: String,
    pub kind: DownloaderKind,
    pub url: String,
    pub username: String,
    pub save_path: String,
    pub password_configured: bool,
    pub is_active: bool,
}
