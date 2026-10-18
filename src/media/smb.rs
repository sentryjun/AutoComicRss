// SPDX-License-Identifier: GPL-2.0-only
//! Read-only SMB access. Never copy raw protocol errors (which can contain paths) to responses.
use super::{validate_relative, DirectoryEntry, DirectoryListing, MediaFile};
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use smb2::{ClientConfig, ErrorKind, FileAttributes, FileInfo, SmbClient, Tree};
use std::{net::IpAddr, time::Duration};
use tokio::sync::Semaphore;

static CONNECTIONS: Semaphore = Semaphore::const_new(4);
const LIST_LIMIT: usize = 1000;
const LOOKUP_LIMIT: usize = 10_000;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SmbConfig {
    pub server: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub share: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}
fn default_port() -> u16 {
    445
}
fn default_timeout() -> u64 {
    15
}

#[derive(Serialize)]
pub struct SmbView {
    pub server: String,
    pub port: u16,
    pub share: String,
    pub username: String,
    pub domain: String,
    pub timeout_seconds: u64,
    pub password_configured: bool,
}
impl SmbConfig {
    pub(crate) fn validate(&mut self) -> Result<()> {
        self.server = self.server.trim().to_owned();
        let valid_host = self.server.parse::<IpAddr>().is_ok()
            || (!self.server.is_empty()
                && self.server.len() <= 253
                && self.server.split('.').all(|label| {
                    !label.is_empty()
                        && label.len() <= 63
                        && !label.starts_with('-')
                        && !label.ends_with('-')
                        && label
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
                }));
        validate_path(&self.share, false)?;
        if !valid_host
            || self.port == 0
            || self.share.contains('/')
            || self.share.len() > 255
            || self.username.trim().is_empty()
            || self.username.eq_ignore_ascii_case("guest")
            || self.username.len() > 1024
            || self.username.chars().any(char::is_control)
            || self.password.is_empty()
            || self.password.len() > 4096
            || self.password.contains('\0')
            || self.domain.len() > 255
            || self.domain.chars().any(char::is_control)
            || !(1..=60).contains(&self.timeout_seconds)
        {
            return Err(Error::Invalid(
                "invalid SMB server, port, share, credentials or timeout (1–60 seconds)".into(),
            ));
        }
        Ok(())
    }
    pub(crate) fn view(&self) -> SmbView {
        SmbView {
            server: self.server.clone(),
            port: self.port,
            share: self.share.clone(),
            username: self.username.clone(),
            domain: self.domain.clone(),
            timeout_seconds: self.timeout_seconds,
            password_configured: !self.password.is_empty(),
        }
    }
    fn deadline(&self) -> Duration {
        Duration::from_secs(self.timeout_seconds)
    }
}

pub(crate) fn validate_path(path: &str, empty: bool) -> Result<()> {
    validate_relative(path, empty)?;
    if path.contains(['*', '?', '"', '<', '>', '|'])
        || path.split('/').any(|p| p.ends_with(['.', ' ']))
    {
        return Err(Error::Invalid("SMB paths must be relative and cannot contain wildcard, stream or ambiguous trailing characters".into()));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Failure {
    Missing,
    Auth,
    Permission,
    Connection,
    Timeout,
    Root,
    Link,
    Invalid,
    Busy,
    Limit,
    Protocol,
}
impl Failure {
    fn code(self) -> &'static str {
        match self {
            Self::Missing => "smb_path_not_found",
            Self::Auth => "smb_authentication_failed",
            Self::Permission => "smb_permission_denied",
            Self::Connection => "smb_connection_failed",
            Self::Timeout => "smb_timeout",
            Self::Root => "smb_root_unavailable",
            Self::Link => "smb_link_or_referral_not_supported",
            Self::Invalid => "smb_invalid_path_or_file_type",
            Self::Busy => "smb_busy",
            Self::Limit => "smb_directory_lookup_limit",
            Self::Protocol => "smb_protocol_error",
        }
    }
}
fn classify(error: smb2::Error) -> Failure {
    match error.kind() {
        ErrorKind::NotFound => Failure::Missing,
        ErrorKind::AuthRequired | ErrorKind::SigningRequired | ErrorKind::GuestSessionRefused => {
            Failure::Auth
        }
        ErrorKind::AccessDenied => Failure::Permission,
        ErrorKind::TimedOut => Failure::Timeout,
        ErrorKind::ConnectionLost | ErrorKind::SessionExpired | ErrorKind::Io => {
            Failure::Connection
        }
        ErrorKind::DfsReferral => Failure::Link,
        _ => Failure::Protocol,
    }
}
type RemoteResult<T> = std::result::Result<T, Failure>;

struct Session {
    client: SmbClient,
    tree: Tree,
}
impl Session {
    async fn connect(config: &SmbConfig) -> RemoteResult<Self> {
        let address = if config.server.contains(':') {
            format!("[{}]:{}", config.server, config.port)
        } else {
            format!("{}:{}", config.server, config.port)
        };
        let mut client = SmbClient::connect(ClientConfig {
            addr: address,
            username: config.username.clone(),
            password: config.password.clone(),
            domain: config.domain.clone(),
            timeout: config.deadline(),
            response_timeout: Some(config.deadline()),
            dfs_enabled: false,
            auto_reconnect: false,
            compression: false,
            ..Default::default()
        })
        .await
        .map_err(classify)?;
        let tree = client
            .connect_share(&config.share)
            .await
            .map_err(|e| match classify(e) {
                Failure::Missing => Failure::Root,
                other => other,
            })?;
        Ok(Self { client, tree })
    }

    // Check each visible component before descending. A server must enforce its own share
    // confinement: server-side links hidden from SMB clients cannot be detected here.
    async fn guard(&mut self, path: &str) -> RemoteResult<()> {
        validate_path(path, true).map_err(|_| Failure::Invalid)?;
        let mut parent = String::new();
        let components: Vec<_> = path.split('/').filter(|c| !c.is_empty()).collect();
        for (index, component) in components.iter().enumerate() {
            let mut reader = self
                .client
                .open_directory_reader(&mut self.tree, &parent)
                .await
                .map_err(classify)?;
            let mut found = None;
            let mut count = 0;
            while let Some(batch) = reader.next_batch().await.map_err(classify)? {
                count += batch.len();
                found = batch
                    .into_iter()
                    .find(|e| e.name == *component || e.name.eq_ignore_ascii_case(component));
                if found.is_some() {
                    break;
                }
                if count >= LOOKUP_LIMIT {
                    return Err(Failure::Limit);
                }
            }
            reader.close().await.map_err(classify)?;
            let entry = found.ok_or(Failure::Missing)?;
            if entry.attributes.contains(FileAttributes::REPARSE_POINT) {
                return Err(Failure::Link);
            }
            if index + 1 < components.len() && !entry.is_directory {
                return Err(Failure::Invalid);
            }
            if !parent.is_empty() {
                parent.push('/');
            }
            parent.push_str(component);
        }
        Ok(())
    }
    async fn stat(&mut self, path: &str) -> RemoteResult<FileInfo> {
        self.guard(path).await?;
        let info = self
            .client
            .stat(&mut self.tree, path)
            .await
            .map_err(classify)?;
        if info.attributes.contains(FileAttributes::REPARSE_POINT) {
            return Err(Failure::Link);
        }
        Ok(info)
    }
    async fn root(&mut self, path: &str) -> RemoteResult<()> {
        match self.stat(path).await {
            Ok(info) if info.is_directory => Ok(()),
            Ok(_) | Err(Failure::Missing) => Err(Failure::Root),
            Err(other) => Err(other),
        }
    }
    async fn close(&mut self) {
        let _ = self.client.disconnect_share(&self.tree).await;
    }
}
fn joined(root: &str, path: &str) -> String {
    match (root.is_empty(), path.is_empty()) {
        (true, _) => path.into(),
        (_, true) => root.into(),
        _ => format!("{root}/{path}"),
    }
}
fn failed_listing(failure: Failure) -> DirectoryListing {
    DirectoryListing {
        status: "unavailable",
        error: Some(failure.code().into()),
        entries: vec![],
        truncated: false,
    }
}

pub(crate) async fn entries(config: &SmbConfig, root: &str, path: &str) -> DirectoryListing {
    let Ok(_permit) = CONNECTIONS.try_acquire() else {
        return failed_listing(Failure::Busy);
    };
    let result = tokio::time::timeout(config.deadline(), async {
        let mut session = Session::connect(config).await?;
        session.root(root).await?;
        let path = joined(root, path);
        session.guard(&path).await?;
        let mut reader = session
            .client
            .open_directory_reader(&mut session.tree, &path)
            .await
            .map_err(classify)?;
        let mut entries = Vec::new();
        let mut truncated = false;
        'listing: while let Some(batch) = reader.next_batch().await.map_err(classify)? {
            for entry in batch {
                if matches!(entry.name.as_str(), "." | "..") {
                    continue;
                }
                if entries.len() == LIST_LIMIT {
                    truncated = true;
                    break 'listing;
                }
                let link = entry.attributes.contains(FileAttributes::REPARSE_POINT);
                entries.push(DirectoryEntry {
                    name: entry.name,
                    kind: if link {
                        "symlink"
                    } else if entry.is_directory {
                        "directory"
                    } else {
                        "file"
                    },
                    size: if link || entry.is_directory {
                        None
                    } else {
                        Some(entry.size)
                    },
                });
            }
        }
        reader.close().await.map_err(classify)?;
        session.close().await;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok::<_, Failure>(DirectoryListing {
            status: "available",
            error: None,
            entries,
            truncated,
        })
    })
    .await;
    match result {
        Ok(Ok(list)) => list,
        Ok(Err(e)) => failed_listing(e),
        Err(_) => failed_listing(Failure::Timeout),
    }
}

fn fail_file(file: &mut MediaFile, failure: Failure) {
    file.status = "unavailable".into();
    file.error = Some(failure.code().into());
    file.observed_size = None;
}
pub(crate) async fn check(
    config: &SmbConfig,
    root: &str,
    mut files: Vec<MediaFile>,
) -> Vec<MediaFile> {
    for file in &mut files {
        file.status = "unchecked".into();
        file.error = None;
        file.observed_size = None;
    }
    let Ok(_permit) = CONNECTIONS.try_acquire() else {
        for file in &mut files {
            fail_file(file, Failure::Busy);
        }
        return files;
    };
    let result = tokio::time::timeout(config.deadline(), async {
        let mut session = Session::connect(config).await?;
        session.root(root).await?;
        for file in &mut files {
            if validate_path(&file.relative_path, false).is_err() {
                fail_file(file, Failure::Invalid);
                continue;
            }
            match session.stat(&joined(root, &file.relative_path)).await {
                Ok(info) if !info.is_directory => {
                    file.status = "present".into();
                    file.observed_size = i64::try_from(info.size).ok();
                }
                Ok(_) => fail_file(file, Failure::Invalid),
                Err(Failure::Missing) => {
                    // A share/root disappearing midway is not evidence that an individual file is absent.
                    match session.root(root).await {
                        Ok(()) => file.status = "absent".into(),
                        Err(e) => fail_file(file, e),
                    }
                }
                Err(e) => fail_file(file, e),
            }
        }
        session.close().await;
        Ok::<_, Failure>(())
    })
    .await;
    let failure = match result {
        Ok(Ok(())) => None,
        Ok(Err(e)) => Some(e),
        Err(_) => Some(Failure::Timeout),
    };
    if let Some(failure) = failure {
        for file in &mut files {
            if file.status == "unchecked" {
                fail_file(file, failure);
            }
        }
    }
    files
}
