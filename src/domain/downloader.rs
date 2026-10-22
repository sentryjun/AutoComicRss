// SPDX-License-Identifier: GPL-2.0-only
use crate::error::{Error, Result};

// Deliberately omit Debug: these fields can contain credentials.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloaderConnection {
    pub url: String,
    pub username: String,
    pub password: String,
    pub save_path: String,
}

impl DownloaderConnection {
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
#[derive(Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloaderInput {
    pub name: String,
    pub kind: DownloaderKind,
    pub connection: DownloaderConnection,
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
