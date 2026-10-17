// SPDX-License-Identifier: GPL-2.0-only
use crate::{
    domain::*,
    error::{Error, Result},
    parser,
};
use reqwest::{Client, Url};
use sha1::{Digest, Sha1};
use std::time::Duration;

pub fn http_client() -> Result<Client> {
    website_client(&crate::config::WebsiteConfig::default())
}

pub fn website_client(config: &crate::config::WebsiteConfig) -> Result<Client> {
    let mut builder = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .user_agent(concat!("rust_project/", env!("CARGO_PKG_VERSION")))
        // Rebuild each redirected request so proxy auth and direct-host rules are reapplied.
        .redirect(reqwest::redirect::Policy::none());
    if let Some(value) = &config.proxy_url {
        let invalid = || {
            Error::Invalid(
                "WEBSITE_PROXY_URL must be an HTTP(S), SOCKS5 or SOCKS5H proxy URL".into(),
            )
        };
        let url = Url::parse(value).map_err(|_| invalid())?;
        if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
            || url.host_str().is_none()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
        {
            return Err(invalid());
        }
        let proxy = reqwest::Proxy::all(url)
            .map_err(|_| invalid())?
            .no_proxy(reqwest::NoProxy::from_string(&config.direct_hosts));
        builder = builder.proxy(proxy);
    }
    Ok(builder.build()?)
}

pub async fn website_get(client: &Client, mut url: Url) -> Result<reqwest::Response> {
    tokio::time::timeout(Duration::from_secs(30), async {
        for hop in 0..=5 {
            validate_http_url(url.as_str())?;
            let response = client.get(url.clone()).send().await?;
            if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                return Ok(response);
            }
            if hop == 5 {
                return Err(Error::Upstream("website redirect limit exceeded".into()));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| Error::Upstream("invalid website redirect".into()))?;
            url = url
                .join(location)
                .map_err(|_| Error::Upstream("invalid website redirect URL".into()))?;
            validate_http_url(url.as_str())
                .map_err(|_| Error::Upstream("unsupported website redirect URL".into()))?;
        }
        unreachable!("redirect count is bounded")
    })
    .await
    .map_err(|_| Error::Upstream("website request timed out".into()))?
}

pub async fn bounded_body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    response = response.error_for_status()?;
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(Error::Upstream("response exceeds size limit".into()));
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if data.len() + chunk.len() > limit {
            return Err(Error::Upstream("response exceeds size limit".into()));
        }
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

pub async fn fetch(client: &Client, sub: &SubscriptionInput) -> Result<Vec<Release>> {
    let response = website_get(
        client,
        Url::parse(&sub.rss_url).map_err(|_| Error::Invalid("invalid RSS URL".into()))?,
    )
    .await?;
    let base = response.url().clone();
    let bytes = bounded_body(response, 4 * 1024 * 1024).await?;
    parse(&bytes, &base, sub)
}

pub fn parse(bytes: &[u8], base: &Url, sub: &SubscriptionInput) -> Result<Vec<Release>> {
    let feed = feed_rs::parser::Builder::new()
        .base_uri(Some(base.as_str()))
        .build()
        .parse(bytes)
        .map_err(|_| Error::Upstream("invalid RSS/Atom feed".into()))?;
    if feed.entries.len() > 2000 {
        return Err(Error::Upstream("feed exceeds 2000 entries".into()));
    }
    Ok(feed
        .entries
        .into_iter()
        .map(|entry| {
            let title = entry.title.map(|t| t.content).unwrap_or_default();
            let media = entry
                .media
                .iter()
                .flat_map(|m| m.content.iter())
                .filter_map(|c| c.url.as_ref().map(|u| u.as_str()));
            let links = entry
                .links
                .iter()
                .filter(|l| {
                    l.rel.as_deref() == Some("enclosure")
                        || l.media_type.as_deref() == Some("application/x-bittorrent")
                        || l.href.starts_with("magnet:")
                        || Url::parse(&l.href)
                            .ok()
                            .is_some_and(|u| u.path().ends_with(".torrent"))
                })
                .map(|l| l.href.as_str());
            let download_url = media.chain(links).find_map(|link| {
                let url = base.join(link).ok()?;
                match url.scheme() {
                    "magnet" => Some(url.to_string()),
                    "http" | "https" if validate_http_url(url.as_str()).is_ok() => {
                        Some(url.to_string())
                    }
                    _ => None,
                }
            });
            let key = format!(
                "{:x}",
                Sha1::digest(download_url.as_deref().unwrap_or(&entry.id).as_bytes())
            );
            let mut release = Release {
                key,
                title,
                download_url,
                episode: None,
                decision: Decision::Ready,
                reason: "matched subscription rules".into(),
            };
            let lower = release.title.to_lowercase();
            if sub
                .exclude
                .iter()
                .any(|v| lower.contains(&v.to_lowercase()))
            {
                release.decision = Decision::Filtered;
                release.reason = "matched an exclusion keyword".into();
            } else if !sub
                .include
                .iter()
                .all(|v| lower.contains(&v.to_lowercase()))
            {
                release.decision = Decision::Filtered;
                release.reason = "missing a required inclusion keyword".into();
            } else if release.download_url.is_none() {
                release.decision = Decision::NeedsReview;
                release.reason = "no supported torrent enclosure or magnet link".into();
            } else {
                match parser::episode(&release.title, sub.season, sub.episode_offset) {
                    Ok(ep) => release.episode = Some(ep),
                    Err(reason) => {
                        release.decision = Decision::NeedsReview;
                        release.reason = reason.into();
                    }
                }
            }
            release
        })
        .collect())
}
