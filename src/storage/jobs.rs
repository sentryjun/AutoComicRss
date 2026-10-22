// SPDX-License-Identifier: GPL-2.0-only
use super::{backend::db_query, now, Store};
use crate::{
    domain::DownloadJob,
    error::{Error, Result},
};

impl Store {
    pub async fn jobs(&self) -> Result<Vec<DownloadJob>> {
        Ok(
            db_query!(self.pool, query_as, "SELECT * FROM download_jobs ORDER BY rowid DESC LIMIT 500"; []; fetch_all)?,
        )
    }

    pub async fn active_jobs(&self) -> Result<Vec<DownloadJob>> {
        Ok(
            db_query!(self.pool, query_as, "SELECT * FROM download_jobs WHERE state NOT IN ('completed', 'missing', 'failed') AND next_attempt_at <= ? ORDER BY rowid"; [now()]; fetch_all)?,
        )
    }

    pub async fn prepare(&self, id: &str, hash: &str, bytes: Option<&[u8]>) -> Result<()> {
        db_query!(self.pool, query, "UPDATE download_jobs SET info_hash = ?, torrent_bytes = ? WHERE id = ?"; [hash, bytes, id]; execute)?;
        Ok(())
    }

    pub async fn torrent_bytes(&self, id: &str) -> Result<Option<Vec<u8>>> {
        Ok(
            db_query!(self.pool, query_scalar, "SELECT torrent_bytes FROM download_jobs WHERE id = ?"; [id]; fetch_one)?,
        )
    }

    pub async fn state(&self, id: &str, state: &str) -> Result<()> {
        db_query!(self.pool, query, "UPDATE download_jobs SET state = ?, last_error = NULL, next_attempt_at = 0 WHERE id = ?"; [state, id]; execute)?;
        Ok(())
    }

    pub async fn failed(&self, job: &DownloadJob, error: &str) -> Result<()> {
        // Keep 'submitting' on an ambiguous network failure, so reconciliation comes first.
        let delay = 30 * (1_i64 << job.attempts.clamp(0, 7));
        db_query!(self.pool, query, "UPDATE download_jobs SET attempts = attempts + 1, last_error = ?, next_attempt_at = ? WHERE id = ?"; [error, now() + delay.min(3600), &job.id]; execute)?;
        Ok(())
    }

    pub async fn retry(&self, id: &str) -> Result<()> {
        let result = db_query!(self.pool, query, "UPDATE download_jobs SET state = 'queued', attempts = 0, next_attempt_at = 0, last_error = NULL WHERE id = ? AND state IN ('failed', 'missing')"; [id]; execute)?;
        if result == 0 {
            return Err(Error::Invalid(
                "only failed or missing jobs can be retried".into(),
            ));
        }
        Ok(())
    }
}
