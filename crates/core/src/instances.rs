// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Registry of instances (user-chosen data folders) and the last one used.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::db;

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
    let reg = load(registry)?.ok_or(Error::Missing(None))?;
    match reg.last {
        None => Err(Error::Missing(None)),
        Some(p) if p.is_dir() => Ok(p),
        Some(p) => Err(Error::Missing(Some(p))),
    }
}

/// Lists `dir` and makes it the last one. A missing or corrupt file is replaced,
/// since the user just chose this folder explicitly. A read error is returned and nothing is
/// written.
pub fn register(registry: &Path, dir: &Path) -> Result<(), Error> {
    let mut reg = match load(registry) {
        Ok(r) => r.unwrap_or_default(),
        Err(Error::Corrupt(_)) => Registry::default(),
        Err(e) => return Err(e),
    };
    if !reg.instances.iter().any(|e| e.path == dir) {
        reg.instances.push(Entry {
            path: dir.to_path_buf(),
        });
    }
    reg.last = Some(dir.to_path_buf());
    save(registry, &reg)
}

fn save(registry: &Path, reg: &Registry) -> Result<(), Error> {
    if let Some(parent) = registry.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(reg).expect("registry always serializes");
    // A crash mid-write would leave a truncated file that then gets replaced, losing the
    // list; rename swaps it in one step on the same volume.
    let tmp = registry.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(tmp, registry)?;
    Ok(())
}

/// Reads the registry; `None` when there is no file yet.
fn load(registry: &Path) -> Result<Option<Registry>, Error> {
    match std::fs::read_to_string(registry) {
        Ok(t) => serde_json::from_str(&t)
            .map(Some)
            .map_err(|_| Error::Corrupt(registry.to_path_buf())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub struct InstanceInfo {
    pub path: PathBuf,
    /// The folder still holds its database.
    pub exists: bool,
}

/// Listed instances in registry order. Never writes, so a corrupt file survives.
pub fn list(registry: &Path) -> Result<Vec<InstanceInfo>, Error> {
    Ok(load(registry)?
        .unwrap_or_default()
        .instances
        .into_iter()
        .map(|e| InstanceInfo {
            exists: has_database(&e.path),
            path: e.path,
        })
        .collect())
}

/// Unlists `dir`; never touches the folder. A corrupt file is left as it is.
pub fn remove(registry: &Path, dir: &Path) -> Result<(), Error> {
    let Some(mut reg) = load(registry)? else {
        return Ok(());
    };
    reg.instances.retain(|e| e.path != dir);
    if reg.last.as_deref() == Some(dir) {
        reg.last = None;
    }
    save(registry, &reg)
}

pub fn has_database(dir: &Path) -> bool {
    dir.join(db::FILE_NAME).is_file()
}

/// Heuristic: inside OneDrive (by its env vars) or a folder named like a sync client's.
pub fn looks_synced(dir: &Path) -> bool {
    let roots: Vec<PathBuf> = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .iter()
        .filter_map(std::env::var_os)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .collect();
    looks_synced_with(dir, &roots)
}

fn looks_synced_with(dir: &Path, synced_roots: &[PathBuf]) -> bool {
    const NAMES: [&str; 4] = ["Dropbox", "Google Drive", "My Drive", "iCloudDrive"];
    synced_roots.iter().any(|r| dir.starts_with(r))
        || dir.components().any(|c| {
            let c = c.as_os_str().to_string_lossy();
            NAMES.iter().any(|n| c.eq_ignore_ascii_case(n))
        })
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
        assert!(!reg.with_extension("json.tmp").exists());
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

    #[test]
    fn register_keeps_the_file_when_it_cannot_be_read() {
        let dir = setup("unreadable");
        let reg = dir.join("instances.json");
        // Reading a directory fails with something other than NotFound.
        std::fs::create_dir_all(&reg).unwrap();
        assert!(matches!(register(&reg, &dir.join("a")), Err(Error::Io(_))));
        assert!(!reg.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn db_folder(dir: &Path, name: &str) -> PathBuf {
        let d = dir.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(db::FILE_NAME), b"").unwrap();
        d
    }

    #[test]
    fn list_keeps_order_and_flags_missing_databases() {
        let dir = setup("list");
        let (reg, a, b) = (
            dir.join("instances.json"),
            db_folder(&dir, "a"),
            dir.join("b"),
        );
        std::fs::create_dir_all(&b).unwrap();
        register(&reg, &a).unwrap();
        register(&reg, &b).unwrap();
        let l = list(&reg).unwrap();
        assert_eq!(
            l.iter()
                .map(|i| (i.path.clone(), i.exists))
                .collect::<Vec<_>>(),
            vec![(a, true), (b, false)]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_without_file_is_empty_and_corrupt_is_an_error() {
        let dir = setup("list-edge");
        let reg = dir.join("instances.json");
        assert!(list(&reg).unwrap().is_empty());
        std::fs::write(&reg, "{not json").unwrap();
        assert!(matches!(list(&reg), Err(Error::Corrupt(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_unlists_and_clears_last() {
        let dir = setup("remove");
        let (reg, a, b) = (dir.join("instances.json"), dir.join("a"), dir.join("b"));
        register(&reg, &a).unwrap();
        register(&reg, &b).unwrap();
        remove(&reg, &a).unwrap();
        assert_eq!(count(&reg), 1);
        assert!(a.parent().unwrap().exists());
        // b is last: removing it leaves no last instance.
        remove(&reg, &b).unwrap();
        assert_eq!(count(&reg), 0);
        assert!(matches!(resolve(&reg), Err(Error::Missing(None))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_keeps_last_when_another_is_removed() {
        let dir = setup("remove-other");
        let (reg, a, b) = (dir.join("instances.json"), dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&b).unwrap();
        register(&reg, &a).unwrap();
        register(&reg, &b).unwrap();
        remove(&reg, &a).unwrap();
        assert_eq!(resolve(&reg).unwrap(), b);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_absent_entry_or_file_is_ok() {
        let dir = setup("remove-absent");
        let reg = dir.join("instances.json");
        remove(&reg, &dir.join("a")).unwrap();
        assert!(!reg.exists());
        register(&reg, &dir.join("a")).unwrap();
        remove(&reg, &dir.join("zzz")).unwrap();
        assert_eq!(count(&reg), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_leaves_a_corrupt_file_untouched() {
        let dir = setup("remove-corrupt");
        let reg = dir.join("instances.json");
        std::fs::write(&reg, "{not json").unwrap();
        assert!(matches!(remove(&reg, &dir), Err(Error::Corrupt(_))));
        assert_eq!(std::fs::read_to_string(&reg).unwrap(), "{not json");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn has_database_checks_the_file() {
        let dir = setup("has-db");
        assert!(!has_database(&dir));
        assert!(has_database(&db_folder(&dir, "a")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn synced_detection() {
        let root = [PathBuf::from("C:/Users/x/OneDrive")];
        assert!(looks_synced_with(
            Path::new("C:/Users/x/OneDrive/data"),
            &root
        ));
        assert!(looks_synced_with(Path::new("C:/Users/x/dropbox/data"), &[]));
        assert!(looks_synced_with(Path::new("G:/My Drive/a"), &[]));
        assert!(looks_synced_with(Path::new("C:/GOOGLE DRIVE/a"), &[]));
        assert!(!looks_synced_with(
            Path::new("C:/Users/x/Documents/data"),
            &root
        ));
        assert!(!looks_synced_with(
            Path::new("C:/Users/x/OneDriveish"),
            &root
        ));
    }
}
