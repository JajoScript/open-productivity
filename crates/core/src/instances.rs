// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Registry of instances (user-chosen data folders) and the last one used.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
struct Registry {
    instances: Vec<Entry>,
    last: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
}

#[derive(Debug)]
pub enum Error {
    /// The registry file exists but cannot be parsed.
    Corrupt(PathBuf),
    /// No registry or no last instance (`None`), or the last folder is gone.
    Missing(Option<PathBuf>),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Corrupt(p) => write!(f, "instance list is corrupt: {}", p.display()),
            Error::Missing(None) => write!(f, "no instance selected"),
            Error::Missing(Some(p)) => write!(f, "instance folder not found: {}", p.display()),
            Error::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

/// Returns the last used instance folder. Never writes, so a corrupt file survives.
pub fn resolve(registry: &Path) -> Result<PathBuf, Error> {
    let text = match std::fs::read_to_string(registry) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Error::Missing(None)),
        Err(e) => return Err(e.into()),
    };
    let reg: Registry =
        serde_json::from_str(&text).map_err(|_| Error::Corrupt(registry.to_path_buf()))?;
    match reg.last {
        None => Err(Error::Missing(None)),
        Some(p) if p.is_dir() => Ok(p),
        Some(p) => Err(Error::Missing(Some(p))),
    }
}

/// Lists `dir` and makes it the last one. A missing or corrupt file is replaced,
/// since the user just chose this folder explicitly.
pub fn register(registry: &Path, dir: &Path) -> Result<(), Error> {
    let mut reg: Registry = std::fs::read_to_string(registry)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    if !reg.instances.iter().any(|e| e.path == dir) {
        reg.instances.push(Entry {
            path: dir.to_path_buf(),
        });
    }
    reg.last = Some(dir.to_path_buf());
    if let Some(parent) = registry.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(&reg).expect("registry always serializes");
    std::fs::write(registry, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("op-inst-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn count(registry: &Path) -> usize {
        let reg: Registry =
            serde_json::from_str(&std::fs::read_to_string(registry).unwrap()).unwrap();
        reg.instances.len()
    }

    #[test]
    fn resolve_without_file_is_missing() {
        let dir = setup("nofile");
        assert!(matches!(
            resolve(&dir.join("instances.json")),
            Err(Error::Missing(None))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn register_then_resolve() {
        let dir = setup("roundtrip");
        let (reg, a) = (dir.join("sub").join("instances.json"), dir.join("a"));
        std::fs::create_dir_all(&a).unwrap();
        register(&reg, &a).unwrap();
        assert_eq!(resolve(&reg).unwrap(), a);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn register_same_dir_twice_lists_once() {
        let dir = setup("twice");
        let (reg, a) = (dir.join("instances.json"), dir.join("a"));
        std::fs::create_dir_all(&a).unwrap();
        register(&reg, &a).unwrap();
        register(&reg, &a).unwrap();
        assert_eq!(count(&reg), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn second_dir_becomes_last_and_both_kept() {
        let dir = setup("second");
        let (reg, a, b) = (dir.join("instances.json"), dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        register(&reg, &a).unwrap();
        register(&reg, &b).unwrap();
        assert_eq!(resolve(&reg).unwrap(), b);
        assert_eq!(count(&reg), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleted_last_folder_is_missing_with_path() {
        let dir = setup("deleted");
        let (reg, a) = (dir.join("instances.json"), dir.join("a"));
        std::fs::create_dir_all(&a).unwrap();
        register(&reg, &a).unwrap();
        std::fs::remove_dir_all(&a).unwrap();
        assert!(matches!(resolve(&reg), Err(Error::Missing(Some(p))) if p == a));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_is_reported_and_untouched() {
        let dir = setup("corrupt");
        let reg = dir.join("instances.json");
        std::fs::write(&reg, "{not json").unwrap();
        assert!(matches!(resolve(&reg), Err(Error::Corrupt(_))));
        assert_eq!(std::fs::read_to_string(&reg).unwrap(), "{not json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn register_replaces_corrupt_file() {
        let dir = setup("replace");
        let (reg, a) = (dir.join("instances.json"), dir.join("a"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(&reg, "garbage").unwrap();
        register(&reg, &a).unwrap();
        assert_eq!(resolve(&reg).unwrap(), a);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
