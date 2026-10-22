// SPDX-License-Identifier: GPL-2.0-only
//! Offline cross-engine copy. No credential-bearing intermediate export files.
use super::backend::{db_query, Pool, Transaction};
use crate::error::{Error, Result};
use futures_util::TryStreamExt;
use sqlx::Row;

#[derive(Clone, Copy)]
enum Kind {
    Text,
    Number,
    Blob,
}
use Kind::{Blob as B, Number as N, Text as T};
struct Table {
    name: &'static str,
    columns: &'static [(&'static str, Kind)],
}
// Stable application columns only: backend migration ledgers, generated digests and locks stay local.
const TABLES: &[Table] = &[
    Table {
        name: "scraping_settings",
        columns: &[("singleton", N), ("config", T)],
    },
    Table {
        name: "subscriptions",
        columns: &[
            ("rowid", N),
            ("id", T),
            ("spec", T),
            ("deleted", N),
            ("last_checked_at", N),
            ("last_error", T),
        ],
    },
    Table {
        name: "downloader_instances",
        columns: &[
            ("rowid", N),
            ("id", T),
            ("name", T),
            ("kind", T),
            ("endpoint", T),
            ("config", T),
        ],
    },
    Table {
        name: "media_destinations",
        columns: &[
            ("rowid", N),
            ("id", T),
            ("name", T),
            ("kind", T),
            ("root_path", T),
            ("smb_config", T),
        ],
    },
    Table {
        name: "destination_mappings",
        columns: &[
            ("rowid", N),
            ("id", T),
            ("destination_id", T),
            ("downloader_id", T),
            ("version", N),
            ("download_root", T),
        ],
    },
    Table {
        name: "releases",
        columns: &[
            ("rowid", N),
            ("subscription_id", T),
            ("resource_key", T),
            ("payload", T),
        ],
    },
    Table {
        name: "download_jobs",
        columns: &[
            ("rowid", N),
            ("id", T),
            ("subscription_id", T),
            ("resource_key", T),
            ("title", T),
            ("season", N),
            ("episode", N),
            ("download_url", T),
            ("torrent_bytes", B),
            ("info_hash", T),
            ("state", T),
            ("attempts", N),
            ("last_error", T),
            ("next_attempt_at", N),
            ("downloader_id", T),
            ("legacy_unassigned", N),
            ("destination_id", T),
            ("mapping_id", T),
            ("download_save_path", T),
            ("file_status", T),
            ("file_checked_at", N),
            ("file_last_success_at", N),
            ("file_error", T),
        ],
    },
    Table {
        name: "media_files",
        columns: &[
            ("rowid", N),
            ("job_id", T),
            ("relative_path", T),
            ("expected_size", N),
            ("observed_size", N),
            ("status", T),
            ("error", T),
        ],
    },
    Table {
        name: "downloader_selection",
        columns: &[("singleton", N), ("downloader_id", T)],
    },
    Table {
        name: "destination_selection",
        columns: &[("singleton", N), ("destination_id", T)],
    },
];
#[derive(PartialEq)]
enum Cell {
    Text(Option<String>),
    Number(Option<i64>),
    Blob(Option<Vec<u8>>),
}
impl Cell {
    fn size(&self) -> usize {
        match self {
            Self::Text(v) => v.as_ref().map_or(0, String::len),
            Self::Number(_) => 8,
            Self::Blob(v) => v.as_ref().map_or(0, Vec::len),
        }
    }
}
type Snapshot = Vec<Vec<Vec<Cell>>>;

macro_rules! read_table {
    ($connection:expr, $sql:expr, $table:expr, $size:expr, $count:expr) => {{
        let mut rows = sqlx::query($sql).fetch($connection);
        let mut data = Vec::new();
        while let Some(row) = rows.try_next().await? {
            let mut cells = Vec::new();
            for (index, (_, kind)) in $table.columns.iter().enumerate() {
                let cell = match kind {
                    Kind::Text => Cell::Text(row.try_get(index)?),
                    Kind::Number => Cell::Number(row.try_get(index)?),
                    Kind::Blob => Cell::Blob(row.try_get(index)?),
                };
                $size += cell.size();
                cells.push(cell);
            }
            $count += 1;
            if $size > 256 * 1024 * 1024 || $count > 100_000 {
                return Err(Error::Invalid(
                    "offline migration exceeds 256 MiB or 100000 row limit".into(),
                ));
            }
            data.push(cells);
        }
        data
    }};
}
async fn read_snapshot(tx: &mut Transaction) -> Result<Snapshot> {
    let mut size = 0;
    let mut count = 0;
    let mut snapshot = Vec::new();
    for table in TABLES {
        let names = table
            .columns
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(",");
        let order = table.columns[0].0;
        let sql = format!("SELECT {names} FROM {} ORDER BY {order}", table.name);
        let data = match tx {
            Transaction::Sqlite(tx) => read_table!(&mut **tx, &sql, table, size, count),
            Transaction::MySql(tx) => read_table!(&mut **tx, &sql, table, size, count),
        };
        snapshot.push(data);
    }
    Ok(snapshot)
}
async fn validate_schema(pool: &Pool) -> Result<()> {
    let ledger: Vec<(i64, bool, Vec<u8>)> = db_query!(pool, query_as, "SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version"; []; fetch_all)?;
    let migrator = match pool {
        Pool::Sqlite(_) => sqlx::migrate!("./migrations/sqlite"),
        Pool::MySql(_) => sqlx::migrate!("./migrations/mysql"),
    };
    if ledger.len() != migrator.iter().count()
        || ledger
            .iter()
            .zip(migrator.iter())
            .any(|((version, success, checksum), migration)| {
                *version != migration.version
                    || !success
                    || checksum.as_slice() != migration.checksum.as_ref()
            })
    {
        return Err(Error::Invalid("source schema must match this application version; upgrade it before offline migration".into()));
    }
    Ok(())
}
async fn preflight_target(pool: &Pool) -> Result<()> {
    let names: Vec<String> = db_query!(pool, query_scalar,
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        "SELECT table_name FROM information_schema.tables WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE'"; []; fetch_all)?;
    for name in names {
        if matches!(name.as_str(), "_sqlx_migrations" | "storage_lock") {
            continue;
        }
        let table = TABLES.iter().find(|t| t.name == name).ok_or_else(|| {
            Error::Invalid(
                "migration target contains unrelated tables; use an empty dedicated database"
                    .into(),
            )
        })?;
        let sql = if table.name.ends_with("_selection") {
            format!(
                "SELECT COUNT(*) FROM {} WHERE {} IS NOT NULL",
                table.name, table.columns[1].0
            )
        } else {
            format!("SELECT COUNT(*) FROM {}", table.name)
        };
        let count: i64 = db_query!(pool, query_scalar, &sql; []; fetch_one)?;
        if count != 0 {
            return Err(Error::Invalid(
                "migration target must be empty; existing data will not be overwritten".into(),
            ));
        }
    }
    Ok(())
}

async fn require_empty(tx: &mut Transaction) -> Result<()> {
    for table in TABLES {
        let sql = if table.name.ends_with("_selection") {
            format!(
                "SELECT COUNT(*) FROM {} WHERE {} IS NOT NULL",
                table.name, table.columns[1].0
            )
        } else {
            format!("SELECT COUNT(*) FROM {}", table.name)
        };
        let count: i64 = db_query!(tx, query_scalar, &sql; []; fetch_one)?;
        if count != 0 {
            return Err(Error::Invalid(
                "migration target must be empty; existing data will not be overwritten".into(),
            ));
        }
    }
    Ok(())
}
macro_rules! insert_row {
    ($db:ty, $connection:expr, $prefix:expr, $row:expr) => {{
        let mut query = sqlx::QueryBuilder::<$db>::new($prefix);
        let mut values = query.separated(",");
        for cell in $row {
            match cell {
                Cell::Text(v) => {
                    values.push_bind(v);
                }
                Cell::Number(v) => {
                    values.push_bind(v);
                }
                Cell::Blob(v) => {
                    values.push_bind(v);
                }
            }
        }
        query.push(")").build().execute($connection).await?;
    }};
}
async fn import(tx: &mut Transaction, snapshot: &Snapshot) -> Result<()> {
    require_empty(tx).await?;
    for (table, rows) in TABLES.iter().zip(snapshot) {
        if table.name.ends_with("_selection") {
            let sql = format!("DELETE FROM {}", table.name);
            db_query!(tx, query, &sql; []; execute)?;
        }
        let names = table
            .columns
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(",");
        let prefix = format!("INSERT INTO {} ({names}) VALUES (", table.name);
        for row in rows {
            match tx {
                Transaction::Sqlite(tx) => insert_row!(sqlx::Sqlite, &mut **tx, &prefix, row),
                Transaction::MySql(tx) => insert_row!(sqlx::MySql, &mut **tx, &prefix, row),
            }
        }
    }
    if &read_snapshot(tx).await? != snapshot {
        return Err(Error::Invalid(
            "migration verification failed; target transaction rolled back".into(),
        ));
    }
    Ok(())
}

/// Both applications must be stopped. Copies only between SQLite and MySQL into an empty target.
/// The source is read-only; target data is committed only after full value-for-value verification.
/// Does not change DATABASE_URL or start a worker. No secrets are returned in the report.
pub async fn migrate_offline(source_url: &str, target_url: &str) -> Result<MigrationReport> {
    if source_url.starts_with("sqlite:") == target_url.starts_with("sqlite:") {
        return Err(Error::Invalid(
            "offline migration requires different database engines".into(),
        ));
    }
    let source = Pool::existing(source_url).await?;
    validate_schema(&source).await?;
    let mut source_tx = source.snapshot().await?;
    let snapshot = read_snapshot(&mut source_tx).await?;
    source_tx.commit().await?;
    source.close().await;
    let target = Pool::target(target_url).await?;
    let result = async {
        // Refuse populated/foreign schemas before running any target DDL.
        preflight_target(&target).await?;
        target.initialize().await?;
        let mut target_tx = target.begin().await?;
        import(&mut target_tx, &snapshot).await?;
        target_tx.commit().await?;
        Ok(MigrationReport {
            rows: snapshot.iter().map(Vec::len).sum(),
        })
    }
    .await;
    target.close().await;
    result
}
#[derive(Debug, serde::Serialize)]
pub struct MigrationReport {
    pub rows: usize,
}
