// SPDX-License-Identifier: GPL-2.0-only
use crate::{
    domain::{episode as parser, *},
    error::{Error, Result},
    network::{bounded_body, website_get},
};
use reqwest::{Client, Url};
use sha1::{Digest, Sha1};

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
