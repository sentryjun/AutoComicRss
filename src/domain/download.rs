// SPDX-License-Identifier: GPL-2.0-only
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Ready,
    Filtered,
    NeedsReview,
    AlreadyQueued,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    pub key: String,
    pub title: String,
    pub download_url: Option<String>,
    pub episode: Option<i32>,
    pub decision: Decision,
    pub reason: String,
    #[serde(default)]
    pub metadata: Option<super::title::ParsedTitle>,
}

#[derive(Clone, Debug, Serialize, sqlx::FromRow)]
pub struct DownloadJob {
    pub id: String,
    pub subscription_id: String,
    pub title: String,
    pub season: i32,
    pub episode: i32,
    pub download_url: String,
    pub info_hash: Option<String>,
    pub state: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub next_attempt_at: i64,
    pub downloader_id: Option<String>,
    pub legacy_unassigned: bool,
    pub destination_id: Option<String>,
    pub mapping_id: Option<String>,
    pub download_save_path: Option<String>,
    pub file_status: String,
    pub file_checked_at: Option<i64>,
    pub file_last_success_at: Option<i64>,
    pub file_error: Option<String>,
}
