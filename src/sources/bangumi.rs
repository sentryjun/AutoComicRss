// SPDX-License-Identifier: GPL-2.0-only
//! Optional anime metadata. API and covers use the website proxy, never LLM credentials.
use crate::{
    domain::validate_http_url,
    error::{Error, Result},
    network::bounded_body,
};
use reqwest::{Client, Url};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore, SemaphorePermit};
const UA: &str = concat!(
    "sentryjun/AutoComicRss/",
    env!("CARGO_PKG_VERSION"),
    " (https://github.com/sentryjun/AutoComicRss)"
);
#[derive(Clone, Serialize)]
pub struct Subject {
    pub id: i64,
    pub title: String,
    pub original_title: String,
    pub summary: String,
    pub date: Option<String>,
    pub episode_count: Option<u64>,
    pub rating: Option<f64>,
    pub cover_url: Option<String>,
    pub page_url: String,
    #[serde(skip)]
    image: Option<Url>,
}
#[derive(Clone)]
enum Cached {
    Subject(Box<Subject>),
    Search(Vec<Subject>),
}
pub struct Bangumi {
    client: Client,
    base: Url,
    requests: Semaphore,
    cache: Mutex<HashMap<String, (Instant, Cached)>>,
}
fn upstream() -> Error {
    Error::Upstream("Bangumi metadata is unavailable; RSS subscriptions can still be used".into())
}
impl Bangumi {
    pub fn new(client: Client, base: &str) -> Result<Self> {
        validate_http_url(base)?;
        let base = Url::parse(base).expect("validated URL");
        if base.query().is_some() || base.fragment().is_some() {
            return Err(Error::Invalid("invalid Bangumi API base URL".into()));
        }
        Ok(Self {
            client,
            base,
            requests: Semaphore::new(4),
            cache: Mutex::new(HashMap::new()),
        })
    }
    async fn permit(&self) -> Result<SemaphorePermit<'_>> {
        tokio::time::timeout(Duration::from_secs(5), self.requests.acquire())
            .await
            .map_err(|_| Error::SourceBusy)?
            .map_err(|_| Error::SourceBusy)
    }
    async fn cached(&self, key: &str) -> Option<Cached> {
        self.cache
            .lock()
            .await
            .get(key)
            .filter(|(at, _)| at.elapsed() < Duration::from_secs(300))
            .map(|(_, v)| v.clone())
    }
    async fn remember(&self, key: String, value: Cached) {
        let mut cache = self.cache.lock().await;
        cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(300));
        if cache.len() >= 256 {
            cache.clear();
        }
        cache.insert(key, (Instant::now(), value));
    }
    fn subject_from(&self, value: &Value) -> Result<Subject> {
        let id = value["id"]
            .as_i64()
            .filter(|id| (1..=i32::MAX as i64).contains(id))
            .ok_or_else(upstream)?;
        if value["type"] != 2 {
            return Err(Error::Invalid("Bangumi subject must be an anime".into()));
        }
        let original_title = value["name"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(upstream)?
            .chars()
            .take(500)
            .collect::<String>();
        let title = value["name_cn"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(&original_title)
            .chars()
            .take(500)
            .collect();
        let image = value["images"]["common"]
            .as_str()
            .or_else(|| value["images"]["medium"].as_str())
            .and_then(|v| Url::parse(v).ok())
            .filter(|u| self.allowed_image(u));
        Ok(Subject {
            id,
            title,
            original_title,
            summary: value["summary"]
                .as_str()
                .unwrap_or_default()
                .chars()
                .take(10000)
                .collect(),
            date: value["date"].as_str().map(|v| v.chars().take(20).collect()),
            episode_count: value["total_episodes"]
                .as_u64()
                .filter(|v| *v > 0)
                .or_else(|| value["eps"].as_u64()),
            rating: value["rating"]["score"]
                .as_f64()
                .filter(|v| (0.0..=10.0).contains(v)),
            cover_url: image
                .as_ref()
                .map(|_| format!("/api/sources/bangumi/subjects/{id}/cover")),
            page_url: format!("https://bgm.tv/subject/{id}"),
            image,
        })
    }
    fn allowed_image(&self, url: &Url) -> bool {
        validate_http_url(url.as_str()).is_ok()
            && url.fragment().is_none()
            && ((url.scheme() == "https"
                && url.host_str() == Some("lain.bgm.tv")
                && url.port_or_known_default() == Some(443))
                || url.origin() == self.base.origin())
    }
    async fn json(&self, request: reqwest::RequestBuilder) -> Result<Value> {
        let response = request
            .header(reqwest::header::USER_AGENT, UA)
            .send()
            .await
            .map_err(|_| upstream())?;
        if !response.status().is_success() {
            return Err(upstream());
        }
        let bytes = bounded_body(response, 2 * 1024 * 1024)
            .await
            .map_err(|_| upstream())?;
        serde_json::from_slice(&bytes).map_err(|_| upstream())
    }
    pub async fn search(&self, query: &str) -> Result<Vec<Subject>> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 200 {
            return Err(Error::Invalid(
                "Bangumi search must contain 1–200 characters".into(),
            ));
        }
        let key = format!("search:{query}");
        if let Some(Cached::Search(subjects)) = self.cached(&key).await {
            return Ok(subjects);
        }
        let _permit = self.permit().await?;
        let value = self
            .json(
                self.client
                    .post(self.base.join("/v0/search/subjects?limit=5").unwrap())
                    .json(&json!({"keyword":query,"sort":"match","filter":{"type":[2]}})),
            )
            .await?;
        let subjects = value["data"]
            .as_array()
            .ok_or_else(upstream)?
            .iter()
            .take(5)
            .map(|v| self.subject_from(v))
            .collect::<Result<Vec<_>>>()?;
        self.remember(key, Cached::Search(subjects.clone())).await;
        Ok(subjects)
    }
    pub async fn subject(&self, id: i64) -> Result<Subject> {
        if !(1..=i32::MAX as i64).contains(&id) {
            return Err(Error::Invalid("invalid Bangumi subject ID".into()));
        }
        let key = format!("subject:{id}");
        if let Some(Cached::Subject(subject)) = self.cached(&key).await {
            return Ok(*subject);
        }
        let _permit = self.permit().await?;
        let value = self
            .json(
                self.client
                    .get(self.base.join(&format!("/v0/subjects/{id}")).unwrap()),
            )
            .await?;
        let subject = self.subject_from(&value)?;
        if subject.id != id {
            return Err(upstream());
        }
        self.remember(key, Cached::Subject(Box::new(subject.clone())))
            .await;
        Ok(subject)
    }
    pub async fn cover(&self, id: i64) -> Result<(&'static str, Vec<u8>)> {
        let mut url = self.subject(id).await?.image.ok_or(Error::NotFound)?;
        let _permit = self.permit().await?;
        // Check each redirect target before requesting it; never proxy arbitrary image URLs.
        for _ in 0..4 {
            if !self.allowed_image(&url) {
                return Err(upstream());
            }
            let response = self
                .client
                .get(url.clone())
                .header(reqwest::header::USER_AGENT, UA)
                .send()
                .await
                .map_err(|_| upstream())?;
            if response.status().is_redirection() {
                url = url
                    .join(
                        response
                            .headers()
                            .get(reqwest::header::LOCATION)
                            .and_then(|v| v.to_str().ok())
                            .ok_or_else(upstream)?,
                    )
                    .map_err(|_| upstream())?;
                continue;
            }
            let bytes = bounded_body(response, 2 * 1024 * 1024)
                .await
                .map_err(|_| upstream())?;
            let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                "image/png"
            } else if bytes.starts_with(b"\xff\xd8\xff") {
                "image/jpeg"
            } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
                "image/gif"
            } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
                "image/webp"
            } else {
                return Err(upstream());
            };
            return Ok((mime, bytes));
        }
        Err(upstream())
    }
}
