// SPDX-License-Identifier: GPL-2.0-only
//! Transmission 3/4 RPC. Only read-only negotiation may fall back to the legacy protocol.
use super::{RemoteFile, RemoteState, RemoteTask};
use crate::{
    config::DownloaderConnection,
    error::{Error, Result},
    network::bounded_body,
};
use reqwest::{header::HeaderValue, Client, StatusCode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

pub(crate) struct Transmission {
    client: Client,
    config: DownloaderConnection,
    session: Option<HeaderValue>,
    modern: bool,
}

fn invalid() -> Error {
    Error::Upstream("invalid Transmission RPC response".into())
}

#[derive(Deserialize)]
struct Task {
    #[serde(alias = "hashString")]
    hash_string: String,
    #[serde(alias = "downloadDir")]
    download_dir: String,
    #[serde(alias = "percentDone")]
    percent_done: f64,
    #[serde(alias = "metadataPercentComplete")]
    metadata_percent_complete: f64,
    status: u8,
    error: u8,
}

impl Transmission {
    pub fn new(config: DownloaderConnection) -> Result<Self> {
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            config,
            session: None,
            modern: true,
        })
    }

    async fn exchange(&mut self, body: Value) -> Result<Value> {
        // A 409 explicitly rejects the request before execution. Other failures are never replayed.
        tokio::time::timeout(Duration::from_secs(30), async {
            for _ in 0..3 {
                let mut request = self.client.post(&self.config.url).json(&body);
                if !self.config.username.is_empty() || !self.config.password.is_empty() {
                    request =
                        request.basic_auth(&self.config.username, Some(&self.config.password));
                }
                if let Some(session) = &self.session {
                    request = request.header("X-Transmission-Session-Id", session);
                }
                let response = request.send().await?;
                if response.status() == StatusCode::CONFLICT {
                    let session = response
                        .headers()
                        .get("X-Transmission-Session-Id")
                        .filter(|value| !value.is_empty() && value.as_bytes().len() <= 1024)
                        .ok_or_else(invalid)?
                        .clone();
                    self.session = Some(session);
                    continue;
                }
                if !response.status().is_success() {
                    return Err(Error::Upstream(format!(
                        "Transmission HTTP request failed ({})",
                        response.status().as_u16()
                    )));
                }
                let bytes = bounded_body(response, 8 * 1024 * 1024).await?;
                return serde_json::from_slice(&bytes).map_err(|_| invalid());
            }
            Err(Error::Upstream(
                "Transmission session negotiation exceeded retry limit".into(),
            ))
        })
        .await
        .map_err(|_| Error::Upstream("Transmission RPC timed out".into()))?
    }

    fn result(&self, response: Value) -> Result<Value> {
        if self.modern {
            if response["jsonrpc"] != "2.0" || response["id"] != 1 {
                return Err(invalid());
            }
            if response.get("error").is_some() {
                return Err(Error::Upstream("Transmission RPC rejected request".into()));
            }
            response
                .get("result")
                .filter(|v| v.is_object())
                .cloned()
                .ok_or_else(invalid)
        } else {
            if response["tag"] != 1 {
                return Err(invalid());
            }
            if response["result"] != "success" {
                return Err(Error::Upstream("Transmission RPC rejected request".into()));
            }
            response
                .get("arguments")
                .filter(|v| v.is_object())
                .cloned()
                .ok_or_else(invalid)
        }
    }

    async fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        let body = if self.modern {
            json!({"jsonrpc":"2.0", "id":1, "method":method,"params":params})
        } else {
            json!({"tag":1,"method":method.replace('_', "-"),"arguments":params})
        };
        let response = self.exchange(body).await?;
        self.result(response)
    }

    pub async fn connect(&mut self) -> Result<()> {
        let response = self.exchange(json!({"jsonrpc":"2.0","id":1,"method":"session_get","params":{"fields":["version"]}})).await?;
        // Old daemons return their bespoke unknown-method response to this read-only probe.
        if response["result"] == "method name not recognized"
            || (response["jsonrpc"] == "2.0"
                && response["id"] == 1
                && response["error"]["code"] == -32601)
        {
            self.modern = false;
            self.rpc("session_get", json!({"fields":["version"]}))
                .await?;
        } else {
            self.result(response)?;
        }
        Ok(())
    }

    fn key<'a>(&self, modern: &'a str, legacy: &'a str) -> &'a str {
        if self.modern {
            modern
        } else {
            legacy
        }
    }

    async fn get(&mut self, hash: Option<&str>, fields: Vec<&str>) -> Result<Vec<Value>> {
        let mut params = json!({"fields":fields});
        if let Some(hash) = hash {
            params["ids"] = json!([hash]);
        }
        let response = self.rpc("torrent_get", params).await?;
        response["torrents"]
            .as_array()
            .filter(|v| v.len() <= 10000)
            .cloned()
            .ok_or_else(invalid)
    }

    pub async fn tasks(&mut self) -> Result<Vec<RemoteTask>> {
        let fields = vec![
            self.key("hash_string", "hashString"),
            self.key("download_dir", "downloadDir"),
            self.key("percent_done", "percentDone"),
            self.key("metadata_percent_complete", "metadataPercentComplete"),
            "status",
            "error",
        ];
        self.get(None, fields)
            .await?
            .into_iter()
            .map(|value| {
                let task: Task = serde_json::from_value(value).map_err(|_| invalid())?;
                if task.hash_string.len() != 40
                    || !task.hash_string.bytes().all(|b| b.is_ascii_hexdigit())
                    || !(0.0..=1.0).contains(&task.percent_done)
                    || !(0.0..=1.0).contains(&task.metadata_percent_complete)
                    || task.status > 6
                    || task.error > 3
                {
                    return Err(invalid());
                }
                // Tracker warnings/errors (1/2) do not mean a local download failed.
                let state = if task.error == 3 {
                    RemoteState::Failed
                } else if task.metadata_percent_complete == 1.0 && task.percent_done == 1.0 {
                    RemoteState::Completed
                } else if task.status == 4 || task.percent_done > 0.0 {
                    RemoteState::Downloading
                } else {
                    RemoteState::Submitted
                };
                Ok(RemoteTask {
                    hash: task.hash_string.to_ascii_lowercase(),
                    state,
                    save_path: Some(task.download_dir),
                })
            })
            .collect()
    }

    pub async fn files(&mut self, hash: &str) -> Result<Vec<RemoteFile>> {
        let key = self.key("hash_string", "hashString");
        let rows = self.get(Some(hash), vec![key, "files", "wanted"]).await?;
        if rows.len() != 1
            || !rows[0][key]
                .as_str()
                .is_some_and(|v| v.eq_ignore_ascii_case(hash))
        {
            return Err(invalid());
        }
        let row = &rows[0];
        let files = row["files"].as_array().ok_or_else(invalid)?;
        let wanted = row["wanted"].as_array().ok_or_else(invalid)?;
        if files.len() > 5000 || files.len() != wanted.len() {
            return Err(invalid());
        }
        files
            .iter()
            .zip(wanted)
            .map(|(file, wanted)| {
                let size = file["length"]
                    .as_i64()
                    .filter(|size| *size >= 0)
                    .ok_or_else(invalid)?;
                let wanted = match wanted {
                    Value::Bool(b) => *b,
                    Value::Number(n) if n.as_u64() == Some(0) => false,
                    Value::Number(n) if n.as_u64() == Some(1) => true,
                    _ => return Err(invalid()),
                };
                Ok(RemoteFile {
                    name: file["name"].as_str().ok_or_else(invalid)?.into(),
                    size,
                    wanted,
                })
            })
            .collect()
    }

    pub async fn submit(
        &mut self,
        hash: &str,
        url: &str,
        bytes: Option<Vec<u8>>,
        save_path: Option<&str>,
    ) -> Result<()> {
        let path = save_path.unwrap_or(&self.config.save_path).to_owned();
        let mut params = json!({"paused":false});
        params[self.key("download_dir", "download-dir")] = json!(path);
        if let Some(bytes) = bytes {
            params["metainfo"] = json!(data_encoding::BASE64.encode(&bytes));
        } else {
            params["filename"] = json!(url);
        }
        let response = self.rpc("torrent_add", params).await?;
        let added = self.key("torrent_added", "torrent-added");
        let duplicate = self.key("torrent_duplicate", "torrent-duplicate");
        let task = response
            .get(added)
            .or_else(|| response.get(duplicate))
            .ok_or_else(invalid)?;
        if !task[self.key("hash_string", "hashString")]
            .as_str()
            .is_some_and(|v| v.eq_ignore_ascii_case(hash))
        {
            return Err(invalid());
        }
        // A concurrent duplicate may already live elsewhere. Do not adopt it or relocate its files.
        let remote = self
            .tasks()
            .await?
            .into_iter()
            .find(|task| task.hash == hash)
            .ok_or_else(invalid)?;
        if !remote
            .save_path
            .as_deref()
            .is_some_and(|actual| crate::media::same_download_root(&path, actual))
        {
            return Err(Error::Invalid(
                "remote save path differs from the submission path; refusing to reuse this torrent"
                    .into(),
            ));
        }
        Ok(())
    }
}
