// SPDX-License-Identifier: GPL-2.0-only
use super::{Kind, Metadata};
use regex::Regex;
use std::sync::LazyLock;
static GROUP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[([^\[\]]{1,100})\]").unwrap());
static RESOLUTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(2160p|1080p|720p|480p|4K)\b").unwrap());
static SEASON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bS([0-9]{1,3})E[0-9]+").unwrap());
static END: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(\s+-\s*\d|\[\d|第\s*\d|\bS\d+E\d|\bEP?\s*\d|\[)").unwrap());
pub(super) fn parse(title: &str, season: i32) -> Metadata {
    let episode = crate::domain::episode::episode(title, season, 0).ok();
    let group = GROUP.captures(title).map(|c| c[1].to_owned());
    let name = GROUP.replace(title, "");
    let end = END.find(&name).map_or(name.len(), |m| m.start());
    let name = name[..end].trim();
    Metadata {
        title: (!name.is_empty()).then(|| name.to_owned()),
        season: SEASON.captures(title).and_then(|c| c[1].parse().ok()),
        episode,
        group,
        resolution: RESOLUTION.captures(title).map(|c| c[1].to_lowercase()),
        kind: if episode.is_some() {
            Kind::Single
        } else {
            Kind::Unknown
        },
    }
}
