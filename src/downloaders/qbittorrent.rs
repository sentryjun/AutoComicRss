// SPDX-License-Identifier: GPL-2.0-only
use crate::{
    config::QbitConfig,
    error::{Error, Result},
    network::bounded_body,
};
use reqwest::{multipart, Client};
use serde::Deserialize;
use std::time::Duration;

#[derive(Clone)]
pub struct Qbit {
    client: Client,
    config: QbitConfig,
}

#[derive(Deserialize)]
pub struct TorrentStatus {
    pub hash: String,
    pub progress: f64,
    pub state: String,
    #[serde(default)]
    pub save_path: Option<String>,
}

#[derive(Deserialize)]
pub struct TorrentFile {
    pub name: String,
    pub size: i64,
    pub priority: i32,
}

impl Qbit {
    pub fn new(config: QbitConfig) -> Result<Self> {
        // Keep downloader cookies in a separate client from untrusted RSS hosts.
        let client = Client::builder()
            .no_proxy()
            .cookie_store(true)
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self { client, config })
    }

    pub async fn login(&self) -> Result<()> {
        let response = self
            .client
            .post(format!("{}/api/v2/auth/login", self.config.url))
            .header("Referer", format!("{}/", self.config.url))
            .form(&[
                ("username", &self.config.username),
                ("password", &self.config.password),
            ])
            .send()
            .await?;
        let bytes = bounded_body(response, 1024).await?;
        if bytes != b"Ok." {
            return Err(Error::Upstream("qBittorrent login failed".into()));
        }
        Ok(())
    }

    pub async fn torrents(&self) -> Result<Vec<TorrentStatus>> {
        let response = self
            .client
            .get(format!("{}/api/v2/torrents/info", self.config.url))
            .header("Referer", format!("{}/", self.config.url))
            .send()
            .await?;
        let bytes = bounded_body(response, 8 * 1024 * 1024).await?;
        serde_json::from_slice(&bytes)
            .map_err(|_| Error::Upstream("invalid qBittorrent task response".into()))
    }

    pub async fn files(&self, hash: &str) -> Result<Vec<TorrentFile>> {
        let response = self
            .client
            .get(format!("{}/api/v2/torrents/files", self.config.url))
            .header("Referer", format!("{}/", self.config.url))
            .query(&[("hash", hash)])
            .send()
            .await?;
        let bytes = bounded_body(response, 8 * 1024 * 1024).await?;
        let files: Vec<TorrentFile> = serde_json::from_slice(&bytes)
            .map_err(|_| Error::Upstream("invalid qBittorrent file list".into()))?;
        if files.len() > 5000 || files.iter().any(|file| file.size < 0) {
            return Err(Error::Upstream(
                "qBittorrent file list exceeds limits or contains invalid sizes".into(),
            ));
        }
        Ok(files)
    }

    pub async fn add(
        &self,
        url: &str,
        bytes: Option<Vec<u8>>,
        save_path: Option<&str>,
    ) -> Result<()> {
        let mut form = multipart::Form::new()
            .text(
                "savepath",
                save_path.unwrap_or(&self.config.save_path).to_owned(),
            )
            .text("tags", "rust-project")
            .text("autoTMM", "false");
        form = if let Some(bytes) = bytes {
            form.part(
                "torrents",
                multipart::Part::bytes(bytes)
                    .file_name("release.torrent")
                    .mime_str("application/x-bittorrent")?,
            )
        } else {
            form.text("urls", url.to_owned())
        };
        let response = self
            .client
            .post(format!("{}/api/v2/torrents/add", self.config.url))
            .header("Referer", format!("{}/", self.config.url))
            .multipart(form)
            .send()
            .await?;
        let bytes = bounded_body(response, 1024).await?;
        if bytes != b"Ok." {
            return Err(Error::Upstream("qBittorrent rejected torrent".into()));
        }
        Ok(())
    }
}
