// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Per-instance key/value settings stored in the instance database (not the app's settings.json).

use rusqlite::{Connection, OptionalExtension};

pub fn get(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .optional()
}

pub fn set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn get_set_and_overwrite() {
        let conn = db::test_conn();
        assert_eq!(get(&conn, "k").unwrap(), None);
        set(&conn, "k", "a").unwrap();
        assert_eq!(get(&conn, "k").unwrap().as_deref(), Some("a"));
        set(&conn, "k", "b").unwrap();
        assert_eq!(get(&conn, "k").unwrap().as_deref(), Some("b"));
    }
}
