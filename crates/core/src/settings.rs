// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! App-level preferences, stored as `settings.json` next to the instance registry.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    /// Follow the operating system language.
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "es")]
    Spanish,
    #[serde(rename = "en")]
    English,
}

impl Language {
    /// The language code of an explicit choice; `None` means "ask the system".
    pub fn code(self) -> Option<&'static str> {
        match self {
            Language::System => None,
            Language::Spanish => Some("es"),
            Language::English => Some("en"),
        }
    }
}

// `default` on the struct lets future settings be missing from old files.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
}

/// Reads the settings. A missing, unreadable or corrupt file gives the defaults and is
/// never written, so the user's file survives until the next explicit `save`.
pub fn load(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, settings: &Settings) -> Result<(), io::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings).expect("settings always serialize");
    // Rename swaps the file in one step, so a crash never leaves a truncated one.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("op-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_is_default() {
        let dir = setup("missing");
        assert_eq!(load(&dir.join("settings.json")).language, Language::System);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trips_each_language() {
        let dir = setup("roundtrip");
        let path = dir.join("sub").join("settings.json");
        for language in [Language::System, Language::Spanish, Language::English] {
            save(&path, &Settings { language }).unwrap();
            assert_eq!(load(&path).language, language);
        }
        assert!(!path.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stored_as_lowercase_codes() {
        let dir = setup("codes");
        let path = dir.join("settings.json");
        save(
            &path,
            &Settings {
                language: Language::Spanish,
            },
        )
        .unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("\"es\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_or_unknown_file_is_default_and_untouched() {
        let dir = setup("corrupt");
        let path = dir.join("settings.json");
        for text in ["{not json", r#"{"language":"klingon"}"#] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(load(&path), Settings::default());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unknown_extra_fields_are_tolerated() {
        let dir = setup("extra");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"language":"en","theme":"dark"}"#).unwrap();
        assert_eq!(load(&path).language, Language::English);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codes() {
        assert_eq!(Language::System.code(), None);
        assert_eq!(Language::Spanish.code(), Some("es"));
        assert_eq!(Language::English.code(), Some("en"));
    }
}
