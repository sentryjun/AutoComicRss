// SPDX-License-Identifier: GPL-2.0-only
//! Media destinations, path mappings and file presence reports.
mod local;
mod path;

pub use local::{check_local, list_local};
pub use path::{normalize_download_root, same_download_root, validate_relative};

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationInput {
    pub name: String,
    pub kind: LocalKind,
    pub root_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocalKind {
    Local,
}

impl DestinationInput {
    pub fn validate(&mut self) -> Result<()> {
        self.name = self.name.trim().to_owned();
        if self.name.is_empty()
            || self.name.chars().count() > 100
            || self.root_path.len() > 4096
            || self.root_path.chars().any(char::is_control)
            || !Path::new(&self.root_path).is_absolute()
            || Path::new(&self.root_path)
                .components()
                .any(|c| matches!(c, Component::ParentDir))
        {
            return Err(Error::Invalid(
                "expected a name and an absolute local root without parent traversal".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Serialize, sqlx::FromRow)]
pub struct Destination {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub root_path: String,
    pub is_active: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingInput {
    pub downloader_id: String,
    pub download_root: String,
}

#[derive(Clone, Serialize, sqlx::FromRow)]
pub struct Mapping {
    pub id: String,
    pub destination_id: String,
    pub downloader_id: String,
    pub version: i64,
    pub download_root: String,
}

#[derive(Clone, Serialize, sqlx::FromRow)]
pub struct MediaFile {
    pub relative_path: String,
    pub expected_size: i64,
    pub observed_size: Option<i64>,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct FileReport {
    pub job_id: String,
    pub destination_id: Option<String>,
    pub status: String,
    pub checked_at: Option<i64>,
    pub last_success_at: Option<i64>,
    pub error: Option<String>,
    pub files: Vec<MediaFile>,
}

#[derive(Serialize)]
pub struct DirectoryEntry {
    pub name: String,
    pub kind: &'static str,
    pub size: Option<u64>,
}
#[derive(Serialize)]
pub struct DirectoryListing {
    pub status: &'static str,
    pub error: Option<String>,
    pub entries: Vec<DirectoryEntry>,
    pub truncated: bool,
}
