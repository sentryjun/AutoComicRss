// SPDX-License-Identifier: GPL-2.0-only
//! Parsed resource titles and conservative episode validation.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Regex,
    Llm,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Single,
    Batch,
    Special,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub title: Option<String>,
    pub season: Option<i32>,
    pub episode: Option<i32>,
    pub group: Option<String>,
    pub resolution: Option<String>,
    pub kind: Kind,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParsedTitle {
    pub source: Mode,
    #[serde(flatten)]
    pub metadata: Metadata,
}

impl ParsedTitle {
    pub fn episode(&self, season: i32, offset: i32) -> std::result::Result<i32, &'static str> {
        let m = &self.metadata;
        if m.kind != Kind::Single {
            return Err("batch, special or ambiguous resource requires manual review");
        }
        if m.season.is_some_and(|s| s != season) {
            return Err("title season differs from subscription season");
        }
        let ep = m
            .episode
            .filter(|e| (1..=10000).contains(e))
            .ok_or("episode is missing or outside supported range")?;
        ep.checked_add(offset)
            .filter(|e| (1..=10000).contains(e))
            .ok_or("episode is outside supported range after offset")
    }
}
