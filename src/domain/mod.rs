// SPDX-License-Identifier: GPL-2.0-only
//! Shared models and validation, independent of persistence and external clients.
mod download;
mod downloader;
pub mod episode;
mod subscription;
pub mod title;
mod validation;

pub use download::{Decision, DownloadJob, Release};
pub use downloader::{DownloaderConnection, DownloaderInput, DownloaderKind, DownloaderView};
pub use subscription::{Subscription, SubscriptionInput};
pub use validation::validate_http_url;
