// SPDX-License-Identifier: GPL-2.0-only
//! HTTP request policy and bounded responses shared by external adapters.
use crate::{
    domain::validate_http_url,
    error::{Error, Result},
};
use reqwest::{Client, Url};
use std::time::Duration;

pub fn website_client(config: &crate::config::WebsiteConfig) -> Result<Client> {
    let mut builder = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ))
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
