// SPDX-License-Identifier: GPL-2.0-only
use crate::error::{Error, Result};
use sqlx::{
    mysql::MySqlPoolOptions,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
    ConnectOptions, MySql, MySqlPool, Sqlite, SqlitePool,
};
use std::{str::FromStr, time::Duration};

/// Deployment-selected connection pool. Application workflows use Store operations.
#[derive(Clone)]
pub enum Pool {
    Sqlite(SqlitePool),
    MySql(MySqlPool),
}

pub(super) enum Executor<'a> {
    SqlitePool(&'a SqlitePool),
    MySqlPool(&'a MySqlPool),
    Sqlite(&'a mut sqlx::SqliteConnection),
    MySql(&'a mut sqlx::MySqlConnection),
}
pub(super) enum Transaction {
    Sqlite(sqlx::Transaction<'static, Sqlite>),
    MySql(sqlx::Transaction<'static, MySql>),
}

impl Pool {
    pub async fn close(&self) {
        match self {
            Self::Sqlite(p) => p.close().await,
            Self::MySql(p) => p.close().await,
        }
    }
    /// Explicit access for SQLite migration fixtures and diagnostics.
    pub fn sqlite(&self) -> Option<&SqlitePool> {
        match self {
            Self::Sqlite(p) => Some(p),
            _ => None,
        }
    }
    pub(super) fn executor(&self) -> Executor<'_> {
        match self {
            Self::Sqlite(p) => Executor::SqlitePool(p),
            Self::MySql(p) => Executor::MySqlPool(p),
        }
    }
    pub(super) async fn snapshot(&self) -> Result<Transaction> {
        match self {
            Self::Sqlite(p) => Ok(Transaction::Sqlite(p.begin().await?)),
            Self::MySql(p) => Ok(Transaction::MySql(
                p.begin_with("START TRANSACTION READ ONLY").await?,
            )),
        }
    }
    pub(super) async fn begin(&self) -> Result<Transaction> {
        match self {
            Self::Sqlite(p) => Ok(Transaction::Sqlite(p.begin_with("BEGIN IMMEDIATE").await?)),
            Self::MySql(p) => {
                let mut tx = p.begin().await?;
                // Serialize compound selection/scan/mapping operations across pool connections.
                sqlx::query("SELECT singleton FROM storage_lock WHERE singleton = 1 FOR UPDATE")
                    .fetch_one(&mut *tx)
                    .await?;
                Ok(Transaction::MySql(tx))
            }
        }
    }
    pub(super) async fn open(url: &str) -> Result<Self> {
        Self::connect(url, true, false).await
    }
    pub(super) async fn existing(url: &str) -> Result<Self> {
        Self::connect(url, false, true).await
    }
    pub(super) async fn target(url: &str) -> Result<Self> {
        Self::connect(url, false, false).await
    }
    pub(super) async fn initialize(&self) -> Result<()> {
        match self {
            Self::Sqlite(pool) => sqlx::migrate!("./migrations/sqlite").run(pool).await?,
            Self::MySql(pool) => sqlx::migrate!("./migrations/mysql").run(pool).await?,
        }
        Ok(())
    }
    async fn connect(url: &str, initialize: bool, read_only: bool) -> Result<Self> {
        if url.starts_with("sqlite:") {
            let options = SqliteConnectOptions::from_str(url)?
                .create_if_missing(!read_only)
                .read_only(read_only)
                .foreign_keys(true)
                .busy_timeout(Duration::from_secs(5))
                .disable_statement_logging();
            let options = if initialize {
                options.journal_mode(SqliteJournalMode::Wal)
            } else {
                options
            };
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await?;
            if initialize {
                sqlx::migrate!("./migrations/sqlite").run(&pool).await?;
            }
            Ok(Self::Sqlite(pool))
        } else if url.starts_with("mysql:") {
            let options = sqlx::mysql::MySqlConnectOptions::from_str(url)?
                .charset("utf8mb4")
                .collation("utf8mb4_bin")
                .disable_statement_logging();
            let pool = MySqlPoolOptions::new().max_connections(5).acquire_timeout(Duration::from_secs(15))
                .after_connect(|conn, _| Box::pin(async move {
                    sqlx::query("SET SESSION sql_mode = 'STRICT_ALL_TABLES,NO_ENGINE_SUBSTITUTION', transaction_isolation = 'REPEATABLE-READ'").execute(conn).await?;
                    Ok(())
                })).connect_with(options).await?;
            if initialize {
                sqlx::migrate!("./migrations/mysql").run(&pool).await?;
            }
            Ok(Self::MySql(pool))
        } else {
            Err(Error::Invalid(
                "DATABASE_URL must use sqlite: or mysql:".into(),
            ))
        }
    }
}
impl Transaction {
    pub(super) fn executor(&mut self) -> Executor<'_> {
        match self {
            Self::Sqlite(tx) => Executor::Sqlite(tx),
            Self::MySql(tx) => Executor::MySql(tx),
        }
    }
    pub(super) async fn commit(self) -> Result<()> {
        match self {
            Self::Sqlite(tx) => tx.commit().await?,
            Self::MySql(tx) => tx.commit().await?,
        };
        Ok(())
    }
}

// Keep typed drivers (including booleans, text and blobs), with one copy of each operation.
// SQL differences are explicit pairs at the call site; no runtime SQL rewriting.
macro_rules! db_query {
    ($ex:expr, $kind:ident, $sql:expr; [$($arg:expr),* $(,)?]; $method:ident) => {
        db_query!($ex, $kind, $sql, $sql; [$($arg),*]; $method)
    };
    ($ex:expr, $kind:ident, $sqlite:expr, $mysql:expr; [$($arg:expr),* $(,)?]; execute) => {{
        match ($ex).executor() {
            $crate::storage::backend::Executor::SqlitePool(p) => sqlx::$kind($sqlite)$(.bind($arg))*.execute(p).await.map(|r| r.rows_affected()),
            $crate::storage::backend::Executor::MySqlPool(p) => sqlx::$kind($mysql)$(.bind($arg))*.execute(p).await.map(|r| r.rows_affected()),
            $crate::storage::backend::Executor::Sqlite(p) => sqlx::$kind($sqlite)$(.bind($arg))*.execute(p).await.map(|r| r.rows_affected()),
            $crate::storage::backend::Executor::MySql(p) => sqlx::$kind($mysql)$(.bind($arg))*.execute(p).await.map(|r| r.rows_affected()),
        }
    }};
    ($ex:expr, $kind:ident, $sqlite:expr, $mysql:expr; [$($arg:expr),* $(,)?]; $method:ident) => {{
        match ($ex).executor() {
            $crate::storage::backend::Executor::SqlitePool(p) => sqlx::$kind($sqlite)$(.bind($arg))*.$method(p).await,
            $crate::storage::backend::Executor::MySqlPool(p) => sqlx::$kind($mysql)$(.bind($arg))*.$method(p).await,
            $crate::storage::backend::Executor::Sqlite(p) => sqlx::$kind($sqlite)$(.bind($arg))*.$method(p).await,
            $crate::storage::backend::Executor::MySql(p) => sqlx::$kind($mysql)$(.bind($arg))*.$method(p).await,
        }
    }};
}
pub(super) use db_query;
