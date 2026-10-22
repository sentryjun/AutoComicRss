// SPDX-License-Identifier: GPL-2.0-only
//! AutoComicRss: RSS subscriptions, download automation and media presence checks.
pub mod app;
pub mod config;
pub mod domain;
pub mod downloaders;
pub mod error;
pub mod http;
pub mod media;
mod network;
pub mod scraping;
pub mod service;
pub mod sources;
pub mod storage;
