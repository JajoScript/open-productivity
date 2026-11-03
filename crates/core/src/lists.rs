// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Task lists: create, list and delete.

use std::fmt;

use rusqlite::{Connection, ffi};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListId(pub i64);

#[derive(Debug)]
pub struct List {
    pub id: ListId,
    pub name: String,
}

#[derive(Debug)]
pub enum ListError {
    EmptyName,
    DuplicateName,
    NotFound,
    Sqlite(rusqlite::Error),
}

impl fmt::Display for ListError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListError::EmptyName => write!(f, "the name is empty"),
            ListError::DuplicateName => write!(f, "a list with that name already exists"),
            ListError::NotFound => write!(f, "list not found"),
            ListError::Sqlite(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for ListError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ListError::Sqlite(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for ListError {
    fn from(e: rusqlite::Error) -> Self {
        ListError::Sqlite(e)
    }
}

pub fn create(conn: &Connection, name: &str) -> Result<ListId, ListError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(ListError::EmptyName);
    }
    match conn.execute("INSERT INTO list (name) VALUES (?1)", [name]) {
        Ok(_) => Ok(ListId(conn.last_insert_rowid())),
        Err(rusqlite::Error::SqliteFailure(f, _))
            if f.extended_code == ffi::SQLITE_CONSTRAINT_UNIQUE =>
        {
            Err(ListError::DuplicateName)
        }
        Err(e) => Err(e.into()),
    }
}

/// Alphabetical, ignoring case.
// shortcut: NOCASE only folds ASCII, so "Ñu" and "ñu" count as different names and
// accented names sort after "z"; upgrade with an ICU/Unicode collation if it matters.
pub fn all(conn: &Connection) -> Result<Vec<List>, ListError> {
    let mut stmt = conn.prepare("SELECT id, name FROM list ORDER BY name COLLATE NOCASE, id")?;
    let lists = stmt
        .query_map([], |r| {
            Ok(List {
                id: ListId(r.get(0)?),
                name: r.get(1)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(lists)
}

/// Its tasks are kept, without a list.
pub fn delete(conn: &Connection, id: ListId) -> Result<(), ListError> {
    match conn.execute("DELETE FROM list WHERE id = ?1", [id.0])? {
        0 => Err(ListError::NotFound),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{db, tasks};

    fn setup(name: &str) -> (std::path::PathBuf, Connection) {
        let dir = std::env::temp_dir().join(format!("op-lists-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let conn = db::open(&dir.join("test.db")).unwrap();
        (dir, conn)
    }

    fn names(conn: &Connection) -> Vec<String> {
        all(conn).unwrap().into_iter().map(|l| l.name).collect()
    }

    #[test]
    fn create_trims_name() {
        let (dir, conn) = setup("trim");
        create(&conn, "  Work \n").unwrap();
        assert_eq!(names(&conn), ["Work"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_name_is_rejected() {
        let (dir, conn) = setup("empty");
        assert!(matches!(create(&conn, ""), Err(ListError::EmptyName)));
        assert!(matches!(create(&conn, " \t\n"), Err(ListError::EmptyName)));
        assert!(names(&conn).is_empty());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_name_is_rejected_ignoring_case_and_spaces() {
        let (dir, conn) = setup("dup");
        create(&conn, "Work").unwrap();
        assert!(matches!(
            create(&conn, "work"),
            Err(ListError::DuplicateName)
        ));
        assert!(matches!(
            create(&conn, " WORK "),
            Err(ListError::DuplicateName)
        ));
        assert_eq!(names(&conn), ["Work"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn all_is_alphabetical_ignoring_case() {
        let (dir, conn) = setup("order");
        for n in ["beta", "Alpha", "gamma", "Delta"] {
            create(&conn, n).unwrap();
        }
        assert_eq!(names(&conn), ["Alpha", "beta", "Delta", "gamma"]);
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_keeps_tasks_without_a_list() {
        let (dir, conn) = setup("delete");
        let work = create(&conn, "Work").unwrap();
        let today = "2026-06-15".parse().unwrap();
        let t = tasks::create(&conn, "a", tasks::View::List(work), today).unwrap();
        delete(&conn, work).unwrap();
        assert!(names(&conn).is_empty());
        let now = chrono::DateTime::parse_from_rfc3339("2026-06-15T12:00:00+00:00").unwrap();
        let l = tasks::list(&conn, tasks::View::All, now).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].id, t);
        assert!(l[0].list.is_none());
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_unknown_is_not_found() {
        let (dir, conn) = setup("missing");
        assert!(matches!(
            delete(&conn, ListId(42)),
            Err(ListError::NotFound)
        ));
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
