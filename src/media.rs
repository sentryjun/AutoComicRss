// SPDX-License-Identifier: GPL-2.0-only
use crate::error::{Error, Result};
use cap_std::{ambient_authority, fs::Dir};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Component, Path},
};

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

// Wire paths always use '/' and must be relative on both Unix and Windows.
pub fn validate_relative(path: &str, allow_empty: bool) -> Result<()> {
    if (path.is_empty() && !allow_empty)
        || path.len() > 4096
        || path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
        || (!path.is_empty()
            && path
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | "..")))
    {
        return Err(Error::Invalid(
            "expected a relative path without empty, dot, parent, drive or backslash components"
                .into(),
        ));
    }
    Ok(())
}

// qBittorrent may run on a different OS. Do not interpret its paths using the host OS.
pub fn normalize_download_root(value: &str) -> Result<String> {
    let path = value.replace('\\', "/");
    let drive = path.len() >= 3
        && path.as_bytes()[0].is_ascii_alphabetic()
        && path.as_bytes()[1] == b':'
        && path.as_bytes()[2] == b'/';
    if path.len() > 4096 || path.chars().any(char::is_control) || !(path.starts_with('/') || drive)
    {
        return Err(Error::Invalid(
            "downloader root must be an absolute Unix or Windows path".into(),
        ));
    }
    let rest = if drive {
        &path[3..]
    } else {
        path.trim_start_matches('/')
    };
    if rest.contains(':') || rest.split('/').any(|s| matches!(s, "." | "..")) {
        return Err(Error::Invalid("invalid downloader root components".into()));
    }
    let path = path.trim_end_matches('/');
    Ok(if path.is_empty() {
        "/".into()
    } else if drive && path.len() == 2 {
        format!("{path}/")
    } else {
        path.into()
    })
}

pub fn same_download_root(expected: &str, actual: &str) -> bool {
    match (
        normalize_download_root(expected),
        normalize_download_root(actual),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn access_error(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::PermissionDenied => "access denied or path escapes the configured root",
        io::ErrorKind::NotFound => "configured root or directory is unavailable",
        _ => "filesystem access failed",
    }
    .into()
}

pub fn check_local(root: &str, mut files: Vec<MediaFile>) -> Vec<MediaFile> {
    let dir = Dir::open_ambient_dir(root, ambient_authority());
    for file in &mut files {
        file.observed_size = None;
        file.error = None;
        let result = match &dir {
            Ok(dir) => {
                if validate_relative(&file.relative_path, false).is_err() {
                    file.status = "unavailable".into();
                    file.error = Some("invalid stored relative path".into());
                    continue;
                }
                dir.metadata(&file.relative_path)
            }
            Err(error) => {
                file.status = "unavailable".into();
                file.error = Some(access_error(error));
                continue;
            }
        };
        match result {
            Ok(metadata) if metadata.is_file() => {
                file.status = "present".into();
                file.observed_size = i64::try_from(metadata.len()).ok();
            }
            Ok(_) => {
                file.status = "unavailable".into();
                file.error = Some("expected a regular file".into());
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => file.status = "absent".into(),
            Err(error) => {
                file.status = "unavailable".into();
                file.error = Some(access_error(&error));
            }
        }
    }
    files
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

pub fn list_local(root: &str, path: &str) -> DirectoryListing {
    if let Err(error) = validate_relative(path, true) {
        return DirectoryListing {
            status: "unavailable",
            error: Some(error.to_string()),
            entries: vec![],
            truncated: false,
        };
    }
    let result = (|| -> io::Result<_> {
        let dir = Dir::open_ambient_dir(root, ambient_authority())?;
        let mut entries = Vec::new();
        let mut truncated = false;
        for (index, item) in dir
            .read_dir(if path.is_empty() { "." } else { path })?
            .enumerate()
        {
            if index >= 1000 {
                truncated = true;
                break;
            }
            let item = item?;
            let kind = item.file_type()?;
            entries.push(DirectoryEntry {
                name: item.file_name().to_string_lossy().into_owned(),
                size: if kind.is_file() {
                    Some(item.metadata()?.len())
                } else {
                    None
                },
                kind: if kind.is_symlink() {
                    "symlink"
                } else if kind.is_dir() {
                    "directory"
                } else if kind.is_file() {
                    "file"
                } else {
                    "other"
                },
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok((entries, truncated))
    })();
    match result {
        Ok((entries, truncated)) => DirectoryListing {
            status: "available",
            error: None,
            entries,
            truncated,
        },
        Err(error) => DirectoryListing {
            status: "unavailable",
            error: Some(access_error(&error)),
            entries: vec![],
            truncated: false,
        },
    }
}
