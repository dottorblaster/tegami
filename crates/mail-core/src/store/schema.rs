// Copyright (C) 2026 Tegami contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Schema and migrations.
//!
//! Migrations are embedded at compile time and applied transactionally,
//! tracked through the `PRAGMA user_version` counter.

use rusqlite::Connection;

use super::StoreError;

const MIGRATIONS: &[&str] = &[
    include_str!("migrations/0001_initial.sql"),
    include_str!("migrations/0002_search.sql"),
    include_str!("migrations/0003_remote_content.sql"),
];

pub fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let current: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    for (index, script) in MIGRATIONS.iter().enumerate() {
        let version = index as i64 + 1;
        if version <= current {
            continue;
        }
        let transaction = connection.transaction()?;
        transaction.execute_batch(script)?;
        transaction.pragma_update(None, "user_version", version)?;
        transaction.commit()?;
    }
    Ok(())
}
