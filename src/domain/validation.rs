// SPDX-License-Identifier: GPL-2.0-only
use crate::error::{Error, Result};

pub fn validate_http_url(value: &str) -> Result<()> {
    let url = reqwest::Url::parse(value).map_err(|_| Error::Invalid("invalid URL".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || value.len() > 8192
    {
        return Err(Error::Invalid(
            "expected an HTTP(S) URL without embedded credentials".into(),
        ));
    }
    Ok(())
}
