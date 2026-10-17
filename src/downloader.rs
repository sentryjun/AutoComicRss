// SPDX-License-Identifier: GPL-2.0-only
use crate::{
    config::QbitConfig,
    domain::validate_http_url,
    error::{Error, Result},
    feeds::bounded_body,
};
use bendy::decoding::{Decoder, Object};
use reqwest::{multipart, Client, Url};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::time::Duration;

pub struct PreparedTorrent {
    pub hash: String,
    pub bytes: Option<Vec<u8>>,
}

pub fn magnet_hash(value: &str) -> Result<String> {
    let url = Url::parse(value).map_err(|_| Error::Invalid("invalid magnet URI".into()))?;
    if url.scheme() != "magnet" {
        return Err(Error::Invalid("expected magnet URI".into()));
    }
    for (key, value) in url.query_pairs() {
        if key != "xt" {
            continue;
        }
        if let Some(hash) = value.strip_prefix("urn:btih:") {
            if hash.len() == 40 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Ok(hash.to_ascii_lowercase());
            }
            if hash.len() == 32 {
                if let Ok(bytes) =
                    data_encoding::BASE32.decode(hash.to_ascii_uppercase().as_bytes())
                {
                    return Ok(data_encoding::HEXLOWER.encode(&bytes));
                }
            }
        }
    }
    Err(Error::Invalid(
        "magnet must contain a valid BitTorrent v1 info hash".into(),
    ))
}

pub fn torrent_hash(bytes: &[u8]) -> Result<String> {
    let invalid = |_| Error::Invalid("invalid or unsupported torrent metadata".into());
    let mut decoder = Decoder::new(bytes).with_max_depth(32);
    let mut root = decoder
        .next_object()
        .map_err(invalid)?
        .ok_or_else(|| Error::Invalid("empty torrent metadata".into()))?
        .try_into_dictionary()
        .map_err(invalid)?;
    let mut raw_info = None;
    while let Some((key, object)) = root.next_pair().map_err(invalid)? {
        if key == b"info" {
            raw_info = Some(
                object
                    .try_into_dictionary()
                    .map_err(invalid)?
                    .into_raw()
                    .map_err(invalid)?,
            );
        }
    }
    drop(root);
    if decoder.next_object().map_err(invalid)?.is_some() {
        return Err(Error::Invalid("trailing torrent data".into()));
    }
    let raw =
        raw_info.ok_or_else(|| Error::Invalid("torrent is missing info dictionary".into()))?;
    let mut info_decoder = Decoder::new(raw).with_max_depth(32);
    let mut info = info_decoder
        .next_object()
        .map_err(invalid)?
        .unwrap()
        .try_into_dictionary()
        .map_err(invalid)?;
    let mut v1 = false;
    while let Some((key, object)) = info.next_pair().map_err(invalid)? {
        if let (b"pieces", Object::Bytes(pieces)) = (key, object) {
            v1 = !pieces.is_empty() && pieces.len() % 20 == 0;
        }
    }
    if !v1 {
        return Err(Error::Invalid(
            "only BitTorrent v1/hybrid torrents are supported".into(),
        ));
    }
    // Hash the exact encoded bytes, never a decoded/re-encoded dictionary.
    Ok(format!("{:x}", Sha1::digest(raw)))
}

pub async fn prepare(client: &Client, url: &str) -> Result<PreparedTorrent> {
    if url.starts_with("magnet:") {
        return Ok(PreparedTorrent {
            hash: magnet_hash(url)?,
            bytes: None,
        });
    }
    validate_http_url(url)?;
    let response =
        crate::feeds::website_get(client, Url::parse(url).expect("validated URL")).await?;
    let bytes = bounded_body(response, 4 * 1024 * 1024).await?;
    Ok(PreparedTorrent {
        hash: torrent_hash(&bytes)?,
        bytes: Some(bytes),
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handles_hex_and_base32_magnets() {
        assert_eq!(
            magnet_hash("magnet:?xt=urn:btih:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap(),
            "a".repeat(40)
        );
        assert_eq!(
            magnet_hash("magnet:?xt=urn:btih:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap(),
            "0".repeat(40)
        );
        assert!(magnet_hash("magnet:?xt=urn:btih:bad").is_err());
    }
    #[test]
    fn hashes_original_info_bytes_and_rejects_bad_metadata() {
        let info = b"d6:lengthi1e4:name1:a12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaae";
        let torrent = [b"d4:info".as_slice(), info, b"e"].concat();
        assert_eq!(
            torrent_hash(&torrent).unwrap(),
            format!("{:x}", Sha1::digest(info))
        );
        for bad in [
            b"<html>error</html>".as_slice(),
            b"d4:infodee",
            b"d4:info",
            &[torrent.as_slice(), b"junk"].concat(),
        ] {
            assert!(torrent_hash(bad).is_err());
        }
    }
}
