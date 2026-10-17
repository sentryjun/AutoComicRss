// SPDX-License-Identifier: GPL-2.0-only
//! Public Mikan discovery; HTML details stay inside this adapter.
use crate::{
    domain::{validate_http_url, SubscriptionInput},
    error::{Error, Result},
    network::{bounded_body, website_get},
};
use reqwest::{Client, Url};
use scraper::{Html, Selector};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};

pub struct Mikan {
    client: Client,
    base: Url,
    cache: Mutex<HashMap<String, (Instant, Cached)>>,
    requests: Semaphore,
}

#[derive(Clone)]
enum Cached {
    Search(SearchResult),
    Series(Box<SeriesDetail>),
}

#[derive(Clone, Serialize)]
pub struct SeriesSummary {
    pub series_id: String,
    pub title: String,
    pub page_url: String,
    pub cover_url: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct SearchResult {
    pub source_id: &'static str,
    pub query: String,
    pub series: Vec<SeriesSummary>,
    pub keyword_rss_url: String,
}

#[derive(Clone, Serialize)]
pub struct SubtitleGroup {
    pub group_id: String,
    pub name: String,
    pub rss_url: String,
}

#[derive(Clone, Serialize)]
pub struct SeriesDetail {
    pub source_id: &'static str,
    pub series_id: String,
    pub title: String,
    pub page_url: String,
    pub rss_url: String,
    pub groups: Vec<SubtitleGroup>,
    pub information: Vec<String>,
    pub bangumi_id: Option<String>,
    pub cover_url: Option<String>,
    #[serde(skip)]
    poster: Option<Url>,
}

#[derive(Serialize)]
pub struct Candidate {
    pub source_id: &'static str,
    pub scope: &'static str,
    pub series_id: Option<String>,
    pub group_id: Option<String>,
    pub page_url: String,
    pub rss_url: String,
    pub subscription: Option<SubscriptionInput>,
    pub warnings: Vec<&'static str>,
}

fn text(element: scraper::ElementRef<'_>) -> String {
    element
        .text()
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn parameters(url: &Url) -> Result<HashMap<String, String>> {
    let mut params = HashMap::new();
    for (key, value) in url.query_pairs() {
        if params
            .insert(key.to_ascii_lowercase(), value.into_owned())
            .is_some()
        {
            return Err(Error::Invalid("duplicate URL parameter".into()));
        }
    }
    Ok(params)
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("static selector")
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20 && value.bytes().all(|b| b.is_ascii_digit())
}

impl Mikan {
    pub fn new(client: Client, base: &str) -> Result<Self> {
        validate_http_url(base)?;
        let base = Url::parse(base).expect("validated URL");
        if base.path() != "/" || base.query().is_some() || base.fragment().is_some() {
            return Err(Error::Invalid(
                "MIKAN_BASE_URL must be an HTTP(S) origin without a path or query".into(),
            ));
        }
        Ok(Self {
            client,
            base,
            cache: Mutex::new(HashMap::new()),
            requests: Semaphore::new(4),
        })
    }

    async fn cached(&self, key: &str) -> Option<Cached> {
        self.cache
            .lock()
            .await
            .get(key)
            .filter(|(created, _)| created.elapsed() < Duration::from_secs(60))
            .map(|(_, value)| value.clone())
    }

    async fn remember(&self, key: String, value: Cached) {
        let mut cache = self.cache.lock().await;
        cache.retain(|_, (created, _)| created.elapsed() < Duration::from_secs(60));
        if cache.len() >= 64 {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, (time, _))| *time)
                .map(|(key, _)| key.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(key, (Instant::now(), value));
    }

    async fn html(&self, url: Url) -> Result<Html> {
        let response = website_get(&self.client, url).await?;
        let bytes = bounded_body(response, 8 * 1024 * 1024).await?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| Error::Upstream("Mikan returned invalid UTF-8".into()))?;
        Ok(Html::parse_document(text))
    }

    fn same_origin_url(&self, value: &str) -> Result<Url> {
        let url = self
            .base
            .join(value)
            .map_err(|_| Error::Invalid("invalid Mikan URL".into()))?;
        validate_http_url(url.as_str())?;
        if url.origin() != self.base.origin() {
            return Err(Error::Invalid(
                "URL must belong to the configured Mikan origin".into(),
            ));
        }
        Ok(url)
    }

    fn rss_link(
        &self,
        element: scraper::ElementRef<'_>,
        series_id: &str,
    ) -> Option<(Url, Option<String>)> {
        let url = self.same_origin_url(element.value().attr("href")?).ok()?;
        if url.path() != "/RSS/Bangumi" {
            return None;
        }
        let params = parameters(&url).ok()?;
        if params.get("bangumiid").map(String::as_str) != Some(series_id) {
            return None;
        }
        let group = params.get("subgroupid").cloned();
        if group.as_ref().is_some_and(|id| !valid_id(id)) {
            return None;
        }
        Some((url, group))
    }

    pub async fn series(&self, id: &str) -> Result<SeriesDetail> {
        if !valid_id(id) {
            return Err(Error::Invalid("invalid Mikan series ID".into()));
        }
        let key = format!("series:{id}");
        if let Some(Cached::Series(detail)) = self.cached(&key).await {
            return Ok(*detail);
        }
        let _permit = self.requests.try_acquire().map_err(|_| Error::SourceBusy)?;
        let page = self
            .base
            .join(&format!("/Home/Bangumi/{id}"))
            .expect("numeric ID");
        let html = self.html(page.clone()).await?;
        let title = html
            .select(&selector(".bangumi-title"))
            .next()
            .map(text)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                Error::Upstream("Mikan detail page is unrecognized or access is restricted".into())
            })?;
        let rss_url = html
            .select(&selector(".bangumi-title a.mikan-rss[href]"))
            .filter_map(|link| self.rss_link(link, id))
            .find(|(_, group)| group.is_none())
            .map(|(url, _)| url.to_string())
            .ok_or_else(|| Error::Upstream("Mikan series RSS link is missing or invalid".into()))?;
        let mut groups = Vec::new();
        let mut seen = HashSet::new();
        for block in html.select(&selector(".subgroup-text")) {
            let Some((url, Some(group_id))) = block
                .select(&selector("a.mikan-rss[href]"))
                .find_map(|link| self.rss_link(link, id))
            else {
                continue;
            };
            let Some(name) = block
                .select(&selector("a[href*='/Home/PublishGroup/']"))
                .next()
                .map(text)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            if seen.insert(group_id.clone()) {
                groups.push(SubtitleGroup {
                    group_id,
                    name,
                    rss_url: url.into(),
                });
            }
        }
        let information = html.select(&selector(".bangumi-info")).map(text).collect();
        let bangumi_id = html
            .select(&selector(".bangumi-info a[href]"))
            .find_map(|link| {
                let url = Url::parse(link.value().attr("href")?).ok()?;
                if !matches!(url.host_str()?, "bgm.tv" | "bangumi.tv" | "chii.in") {
                    return None;
                }
                url.path()
                    .strip_prefix("/subject/")
                    .filter(|id| valid_id(id))
                    .map(str::to_owned)
            });
        static CSS_URL: OnceLock<regex::Regex> = OnceLock::new();
        let pattern = CSS_URL.get_or_init(|| {
            regex::Regex::new(r#"url\(\s*['"]?([^'"\s)]+)['"]?\s*\)"#).expect("static regex")
        });
        let poster = html
            .select(&selector(".bangumi-poster[style]"))
            .find_map(|element| {
                let captures = pattern.captures(element.value().attr("style")?)?;
                let url = self.same_origin_url(captures.get(1)?.as_str()).ok()?;
                url.path().starts_with("/images/").then_some(url)
            });
        let detail = SeriesDetail {
            source_id: "mikan",
            series_id: id.into(),
            title,
            page_url: page.into(),
            rss_url,
            groups,
            information,
            bangumi_id,
            cover_url: poster
                .as_ref()
                .map(|_| format!("/api/sources/mikan/series/{id}/cover")),
            poster,
        };
        self.remember(key, Cached::Series(Box::new(detail.clone())))
            .await;
        Ok(detail)
    }

    pub async fn cover(&self, id: &str) -> Result<(String, Vec<u8>)> {
        let url = self.series(id).await?.poster.ok_or(Error::NotFound)?;
        let _permit = self.requests.try_acquire().map_err(|_| Error::SourceBusy)?;
        let response = website_get(&self.client, url).await?;
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !matches!(
            mime.as_str(),
            "image/png" | "image/jpeg" | "image/webp" | "image/gif"
        ) {
            return Err(Error::Upstream(
                "Mikan cover is not a supported raster image".into(),
            ));
        }
        Ok((mime, bounded_body(response, 4 * 1024 * 1024).await?))
    }

    pub async fn resolve(&self, value: &str) -> Result<Candidate> {
        if value.len() > 8192 {
            return Err(Error::Invalid("Mikan URL is too long".into()));
        }
        let url = self.same_origin_url(value.trim())?;
        let params = parameters(&url)?;
        if matches!(url.path(), "/RSS/Search" | "/Home/Search") {
            let query = params
                .get("searchstr")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty() && s.chars().count() <= 100)
                .ok_or_else(|| {
                    Error::Invalid("keyword RSS requires a query of 1–100 characters".into())
                })?;
            if params.len() != 1 {
                return Err(Error::Invalid("unsupported keyword RSS parameters".into()));
            }
            let mut rss = self.base.join("/RSS/Search").expect("static path");
            rss.query_pairs_mut().append_pair("searchstr", query);
            let mut page = rss.clone();
            page.set_path("/Home/Search");
            return Ok(Candidate { source_id: "mikan", scope: "keyword", series_id: None, group_id: None,
                page_url: page.into(), rss_url: rss.into(), subscription: None,
                warnings: vec!["Keyword RSS can contain multiple series. Select a specific series, or manually preview with filters that restrict the feed to one series before creating a subscription."] });
        }
        let (id, group) = if let Some(id) = url.path().strip_prefix("/Home/Bangumi/") {
            (id.to_owned(), None)
        } else if url.path() == "/RSS/Bangumi" {
            (
                params
                    .get("bangumiid")
                    .cloned()
                    .ok_or_else(|| Error::Invalid("missing bangumiId".into()))?,
                params.get("subgroupid").cloned(),
            )
        } else {
            return Err(Error::Invalid(
                "expected a Mikan series page or series RSS URL".into(),
            ));
        };
        if group.as_ref().is_some_and(|id| !valid_id(id)) {
            return Err(Error::Invalid("invalid subtitle group ID".into()));
        }
        let detail = self.series(&id).await?;
        let rss_url = match &group {
            Some(group) => detail
                .groups
                .iter()
                .find(|g| &g.group_id == group)
                .map(|g| g.rss_url.clone())
                .ok_or_else(|| {
                    Error::Invalid("subtitle group is not available for this series".into())
                })?,
            None => detail.rss_url,
        };
        Ok(Candidate {
            source_id: "mikan", scope: if group.is_some() { "group" } else { "series" },
            series_id: Some(id), group_id: group, page_url: detail.page_url, rss_url: rss_url.clone(),
            subscription: Some(SubscriptionInput { title: detail.title, rss_url, season: 1, episode_offset: 0,
                include: Vec::new(), exclude: Vec::new(), enabled: false }),
            warnings: vec!["Confirm season, episode offset and filters in preview before enabling; season 1 is a default, not inferred metadata."],
        })
    }

    pub async fn search(&self, query: &str) -> Result<SearchResult> {
        let query = query.trim();
        if query.is_empty() || query.chars().count() > 100 {
            return Err(Error::Invalid(
                "search query must contain 1–100 characters".into(),
            ));
        }
        let key = format!("search:{query}");
        if let Some(Cached::Search(result)) = self.cached(&key).await {
            return Ok(result);
        }
        let _permit = self.requests.try_acquire().map_err(|_| Error::SourceBusy)?;
        let mut url = self.base.join("/Home/Search").expect("static path");
        url.query_pairs_mut().append_pair("searchstr", query);
        let html = self.html(url.clone()).await?;
        let unrecognized =
            || Error::Upstream("Mikan search page is unrecognized or access is restricted".into());
        let search_form = html
            .select(&selector(
                "form[action='/Home/Search'] input[name='searchstr']",
            ))
            .next()
            .is_some();
        let results = html.select(&selector(".an-ul")).next().is_some()
            || html
                .select(&selector(".header2-text"))
                .any(|element| text(element).contains("搜索结果"));
        if !search_form || !results {
            return Err(unrecognized());
        }
        let mut seen = HashSet::new();
        let mut series = Vec::new();
        for link in html.select(&selector(".an-ul a[href]")) {
            let Some(url) = link
                .value()
                .attr("href")
                .and_then(|s| self.base.join(s).ok())
            else {
                continue;
            };
            if url.origin() != self.base.origin() {
                continue;
            }
            let Some(id) = url
                .path()
                .strip_prefix("/Home/Bangumi/")
                .filter(|id| valid_id(id))
            else {
                continue;
            };
            let Some(name) = link.select(&selector(".an-text")).next() else {
                return Err(unrecognized());
            };
            let text = name.text().collect::<String>();
            let title = name.value().attr("title").unwrap_or(&text).trim();
            if title.is_empty() {
                return Err(unrecognized());
            }
            if !seen.insert(id.to_owned()) {
                continue;
            }
            series.push(SeriesSummary {
                series_id: id.into(),
                title: title.into(),
                page_url: url.to_string(),
                cover_url: link
                    .select(&selector("[data-src], img[src]"))
                    .next()
                    .map(|_| format!("/api/sources/mikan/series/{id}/cover")),
            });
        }
        url.set_path("/RSS/Search");
        let result = SearchResult {
            source_id: "mikan",
            query: query.into(),
            series,
            keyword_rss_url: url.into(),
        };
        self.remember(key, Cached::Search(result.clone())).await;
        Ok(result)
    }
}
