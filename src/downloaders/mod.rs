// SPDX-License-Identifier: GPL-2.0-only
//! Downloader protocols and torrent metadata preparation.
pub mod qbittorrent;
pub mod torrent;
mod transmission;

use crate::{config::DownloaderConnection, domain::DownloaderKind, error::Result};

/// Both protocols accept a v1 info hash as a stable remote identifier.
/// Transmission's numeric IDs are deliberately not persisted: they can change on restart.
pub(crate) struct RemoteTask {
    pub hash: String,
    pub state: RemoteState,
    pub save_path: Option<String>,
}

pub(crate) enum RemoteState {
    Submitted,
    Downloading,
    Completed,
    Failed,
}

impl RemoteState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Downloading => "downloading",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

pub(crate) struct RemoteFile {
    pub name: String,
    pub size: i64,
    pub wanted: bool,
}

pub(crate) enum Downloader {
    Qbit(qbittorrent::Qbit),
    Transmission(transmission::Transmission),
}

impl Downloader {
    pub fn new(kind: DownloaderKind, config: DownloaderConnection) -> Result<Self> {
        match kind {
            DownloaderKind::Qbittorrent => Ok(Self::Qbit(qbittorrent::Qbit::new(config)?)),
            DownloaderKind::Transmission => {
                Ok(Self::Transmission(transmission::Transmission::new(config)?))
            }
        }
    }

    pub async fn connect(&mut self) -> Result<()> {
        match self {
            Self::Qbit(client) => client.login().await,
            Self::Transmission(client) => client.connect().await,
        }
    }

    pub async fn tasks(&mut self) -> Result<Vec<RemoteTask>> {
        match self {
            Self::Qbit(client) => client.tasks().await,
            Self::Transmission(client) => client.tasks().await,
        }
    }

    pub async fn files(&mut self, hash: &str) -> Result<Vec<RemoteFile>> {
        match self {
            Self::Qbit(client) => Ok(client
                .files(hash)
                .await?
                .into_iter()
                .map(|file| RemoteFile {
                    name: file.name,
                    size: file.size,
                    wanted: file.priority != 0,
                })
                .collect()),
            Self::Transmission(client) => client.files(hash).await,
        }
    }

    pub async fn submit(
        &mut self,
        hash: &str,
        url: &str,
        bytes: Option<Vec<u8>>,
        save_path: Option<&str>,
    ) -> Result<()> {
        match self {
            Self::Qbit(client) => client.add(url, bytes, save_path).await,
            Self::Transmission(client) => client.submit(hash, url, bytes, save_path).await,
        }
    }
}
