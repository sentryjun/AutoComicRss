// SPDX-License-Identifier: GPL-2.0-only
use super::{validate_relative, DirectoryEntry, DirectoryListing, MediaFile};
use cap_std::{ambient_authority, fs::Dir};
use std::io;

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
