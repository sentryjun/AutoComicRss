// SPDX-License-Identifier: GPL-2.0-only
//! Deployment-selected persistence with shared transactional operations.
mod backend;
mod downloaders;
mod jobs;
mod media;
mod scraping;
mod subscriptions;
mod transfer;

use crate::error::Result;
use backend::db_query;
pub use backend::Pool;
use std::time::{SystemTime, UNIX_EPOCH};
pub use transfer::{migrate_offline, MigrationReport};

#[derive(Clone)]
pub struct Store {
    pub pool: Pool,
}

pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

impl Store {
    pub async fn open(url: &str) -> Result<Self> {
        Ok(Self {
            pool: Pool::open(url).await?,
        })
    }
    pub async fn ping(&self) -> Result<()> {
        let _: i64 = db_query!(self.pool, query_scalar, "SELECT 1"; []; fetch_one)?;
        Ok(())
    }
}

// Ignore only a duplicate key. Foreign keys, truncation and invalid data must roll back.
fn ignore_duplicate(result: std::result::Result<u64, sqlx::Error>) -> crate::error::Result<u64> {
    match result {
        Ok(rows) => Ok(rows),
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => Ok(0),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests;
