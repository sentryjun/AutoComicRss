// SPDX-License-Identifier: GPL-2.0-only
//! Deployment-selected persistence with shared transactional operations.
mod backend;
mod media;
mod operations;
mod transfer;
pub use backend::Pool;
pub use operations::Store;
pub use transfer::{migrate_offline, MigrationReport};

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
