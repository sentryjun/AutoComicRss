// SPDX-License-Identifier: GPL-2.0-only
//! Transactional persistence; SQLite owns connections, SQL and migrations.
mod sqlite;

pub use sqlite::Store;
