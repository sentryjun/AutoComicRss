// SPDX-License-Identifier: GPL-2.0-only
//! One globally selected parser; uncertain results never become download jobs.
mod engine;
mod llm;
mod regex;
mod settings;

pub use crate::domain::title::{Kind, Metadata, Mode, ParsedTitle};
pub use engine::Scraper;
pub use settings::{Settings, SettingsInput};
