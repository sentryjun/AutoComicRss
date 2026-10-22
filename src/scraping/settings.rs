// SPDX-License-Identifier: GPL-2.0-only
use super::Mode;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

// Credential-bearing persistence type deliberately has no Debug implementation.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    pub mode: Mode,
    pub base_url: String,
    pub model: String,
    pub(crate) api_key: String,
    pub(crate) revision: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsInput {
    pub mode: Mode,
    pub base_url: String,
    pub model: String,
    /// Omitted/null keeps the existing key; an empty string clears it.
    pub api_key: Option<String>,
}
impl Settings {
    pub fn view(&self) -> serde_json::Value {
        serde_json::json!({"mode":self.mode,"base_url":self.base_url,"model":self.model,
            "api_key_configured":!self.api_key.is_empty(),"max_requests_per_preview":20,"timeout_seconds":30})
    }
    pub fn updated(self, input: SettingsInput) -> Result<Self> {
        let base_url = input.base_url.trim().trim_end_matches('/').to_owned();
        if !base_url.is_empty() {
            crate::domain::validate_http_url(&base_url)?;
            let url = reqwest::Url::parse(&base_url).expect("validated URL");
            if url.query().is_some() || url.fragment().is_some() || base_url.len() > 2048 {
                return Err(Error::Invalid(
                    "LLM API base URL cannot contain query parameters or a fragment".into(),
                ));
            }
        }
        if base_url != self.base_url && !self.api_key.is_empty() && input.api_key.is_none() {
            return Err(Error::Invalid(
                "re-enter or clear the API key when changing the LLM endpoint".into(),
            ));
        }
        let key = input.api_key.unwrap_or(self.api_key);
        let model = input.model.trim().to_owned();
        if key.len() > 4096
            || !key.bytes().all(|c| (33..=126).contains(&c))
            || model.len() > 200
            || model.chars().any(char::is_control)
        {
            return Err(Error::Invalid("invalid LLM model or API key".into()));
        }
        if input.mode == Mode::Llm && (base_url.is_empty() || model.is_empty() || key.is_empty()) {
            return Err(Error::Invalid(
                "LLM mode requires an API base URL, model and API key".into(),
            ));
        }
        Ok(Self {
            mode: input.mode,
            base_url,
            model,
            api_key: key,
            revision: uuid::Uuid::new_v4().to_string(),
        })
    }
}
