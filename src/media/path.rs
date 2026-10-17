// SPDX-License-Identifier: GPL-2.0-only
use crate::error::{Error, Result};

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
