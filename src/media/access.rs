// SPDX-License-Identifier: GPL-2.0-only
use super::{local, smb, DirectoryListing, MediaFile, SmbConfig};
use crate::error::{Error, Result};

/// Each request owns its access configuration. Global selection changes never retarget it.
pub(crate) enum MediaAccess {
    Local(String),
    Smb { root: String, config: SmbConfig },
}

impl MediaAccess {
    pub async fn entries(self, path: &str) -> Result<DirectoryListing> {
        match self {
            Self::Local(root) => {
                let path = path.to_owned();
                tokio::task::spawn_blocking(move || local::list_local(&root, &path))
                    .await
                    .map_err(|_| Error::Upstream("local directory query failed".into()))
            }
            Self::Smb { root, config } => {
                smb::validate_path(path, true)?;
                Ok(smb::entries(&config, &root, path).await)
            }
        }
    }

    pub async fn check(self, files: Vec<MediaFile>) -> Result<Vec<MediaFile>> {
        match self {
            Self::Local(root) => {
                tokio::task::spawn_blocking(move || local::check_local(&root, files))
                    .await
                    .map_err(|_| Error::Upstream("local file check failed".into()))
            }
            Self::Smb { root, config } => Ok(smb::check(&config, &root, files).await),
        }
    }
}
