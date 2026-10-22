// SPDX-License-Identifier: GPL-2.0-only
use super::validate_http_url;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionInput {
    pub title: String,
    pub rss_url: String,
    #[serde(default = "one")]
    pub season: i32,
    #[serde(default)]
    pub episode_offset: i32,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    // Opt in after previewing: the first scan considers all entries in the feed.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub bangumi_id: Option<i64>,
}

fn one() -> i32 {
    1
}

impl SubscriptionInput {
    pub fn validate(&mut self) -> Result<()> {
        self.title = self.title.trim().to_owned();
        self.rss_url = self.rss_url.trim().to_owned();
        if self.title.is_empty() || self.title.chars().count() > 200 {
            return Err(Error::Invalid("title must contain 1–200 characters".into()));
        }
        validate_http_url(&self.rss_url)?;
        if self
            .bangumi_id
            .is_some_and(|id| !(1..=i32::MAX as i64).contains(&id))
        {
            return Err(Error::Invalid("invalid Bangumi subject ID".into()));
        }
        if !(1..=999).contains(&self.season) || self.episode_offset.abs_diff(0) > 10000 {
            return Err(Error::Invalid(
                "season or episode_offset is out of range".into(),
            ));
        }
        for rules in [&mut self.include, &mut self.exclude] {
            if rules.len() > 32 {
                return Err(Error::Invalid("at most 32 rules per list".into()));
            }
            for rule in rules {
                *rule = rule.trim().to_owned();
                if rule.is_empty() || rule.len() > 200 {
                    return Err(Error::Invalid(
                        "filter keywords must contain 1–200 bytes".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subscription {
    pub id: String,
    #[serde(flatten)]
    pub input: SubscriptionInput,
    pub last_checked_at: Option<i64>,
    pub last_error: Option<String>,
}
