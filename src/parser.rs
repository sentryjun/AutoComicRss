// SPDX-License-Identifier: GPL-2.0-only
use regex::Regex;
use std::{collections::BTreeSet, sync::LazyLock};

static UNSUPPORTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
    r"(?i)(合集|全集|\bbatch\b|\bcomplete\b|\b(?:OVA|SP|NCOP|NCED)(?:[0-9]+\b|\b)|(?:\s-\s*|\[|第|\bEP?\s*)[0-9]+\.[0-9]+|[0-9]+\s*[-~～]\s*[0-9]+)"
).unwrap()
});
static SEASON_EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bS([0-9]{1,3})E([0-9]{1,4})(?:v[0-9]+)?(?:\b|_)").unwrap());
static EPISODE: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)\[([0-9]{1,3})(?:v[0-9]+)?(?:\s*END)?\]",
        r"第\s*([0-9]{1,4})\s*[话話集]",
        r"(?i)\bEP?\s*([0-9]{1,4})(?:v[0-9]+)?\b",
        r"(?i)\s-\s*([0-9]{1,4})(?:v[0-9]+)?(?:\s|\[|$)",
    ]
    .into_iter()
    .map(|r| Regex::new(r).unwrap())
    .collect()
});

/// Conservative single-episode parsing. Ambiguity is visible, never guessed.
pub fn episode(title: &str, season: i32, offset: i32) -> std::result::Result<i32, &'static str> {
    if UNSUPPORTED.is_match(title) {
        return Err("batch, special episode or fractional episode requires manual review");
    }
    let mut candidates = BTreeSet::new();
    for captures in SEASON_EPISODE.captures_iter(title) {
        if captures[1].parse::<i32>().ok() != Some(season) {
            return Err("title season differs from subscription season");
        }
        candidates.insert(captures[2].parse::<i32>().unwrap());
    }
    for pattern in EPISODE.iter() {
        for captures in pattern.captures_iter(title) {
            candidates.insert(captures[1].parse::<i32>().unwrap());
        }
    }
    if candidates.len() != 1 {
        return Err("episode is missing or ambiguous");
    }
    let value = candidates
        .first()
        .copied()
        .unwrap()
        .checked_add(offset)
        .ok_or("episode offset overflow")?;
    if !(1..=10000).contains(&value) {
        return Err("episode is outside supported range after offset");
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_episode_formats() {
        for title in [
            "[Group] Example [03][1080p]",
            "Example - 03v2 [1080p]",
            "Example 第03话",
            "Example S01E03",
            "Example EP03",
            "Example - 03 [1080p HEVC AAC2.0]",
        ] {
            assert_eq!(episode(title, 1, 0), Ok(3), "{title}");
        }
        assert_eq!(episode("Example - 13 [1080p]", 2, -12), Ok(1));
    }
    #[test]
    fn uncertain_titles_are_not_downloaded() {
        for title in [
            "Example [1080p]",
            "Example [01-12]",
            "Example - 12.5",
            "Example SP01",
            "Example SP01 [03]",
            "Example [03] [04]",
            "Example S02E03",
            "Example S01E03E04",
        ] {
            assert!(episode(title, 1, 0).is_err(), "{title}");
        }
        assert!(episode("Example [01]", 1, -1).is_err());
    }
}
