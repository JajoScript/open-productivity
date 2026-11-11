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

/// Light/dark appearance. Not called `Theme`: that is the Slint global.
#[derive(Serialize, Deserialize, Default, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    /// Follow the operating system.
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "light")]
    Light,
    #[serde(rename = "dark")]
    Dark,
}

// `default` on the struct lets future settings be missing from old files.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
    pub color_mode: ColorMode,
    /// Ids of the unchecked calendar sources. Text, so an id from a newer version does not make
    /// the whole file unreadable and survives each save.
    pub hidden_calendars: Vec<String>,
    /// The Pomodoro length last used to study; `None` is Flowtime.
    pub study_preset: Option<crate::study::Preset>,
}

/// What the Calendar can show; each source has its own checkbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalendarSource {
    Tasks,
    Habits,
}

impl CalendarSource {
    pub const ALL: [CalendarSource; 2] = [CalendarSource::Tasks, CalendarSource::Habits];

    fn id(self) -> &'static str {
        match self {
            CalendarSource::Tasks => "tasks",
            CalendarSource::Habits => "habits",
        }
    }
}

impl Settings {
    pub fn shows(&self, source: CalendarSource) -> bool {
        !self.hidden_calendars.iter().any(|h| h == source.id())
    }

    pub fn set_shown(&mut self, source: CalendarSource, shown: bool) {
        self.hidden_calendars.retain(|h| h != source.id());
        if !shown {
            self.hidden_calendars.push(source.id().to_string());
        }
    }
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
            save(
                &path,
                &Settings {
                    language,
                    ..Default::default()
                },
            )
            .unwrap();
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
                ..Default::default()
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
        std::fs::write(&path, r#"{"language":"en","font":"big"}"#).unwrap();
        assert_eq!(load(&path).language, Language::English);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trips_each_color_mode() {
        let dir = setup("colormode");
        let path = dir.join("settings.json");
        for color_mode in [ColorMode::System, ColorMode::Light, ColorMode::Dark] {
            let s = Settings {
                color_mode,
                ..Default::default()
            };
            save(&path, &s).unwrap();
            assert_eq!(load(&path).color_mode, color_mode);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_file_without_color_mode_is_system_and_keeps_language() {
        let dir = setup("oldfile");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"language":"es"}"#).unwrap();
        let s = load(&path);
        assert_eq!(s.color_mode, ColorMode::System);
        assert_eq!(s.language, Language::Spanish);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn changing_one_setting_keeps_the_other() {
        let dir = setup("keep");
        let path = dir.join("settings.json");
        let s = Settings {
            language: Language::English,
            color_mode: ColorMode::Dark,
            ..Default::default()
        };
        save(&path, &s).unwrap();
        let mut s = load(&path);
        s.color_mode = ColorMode::Light;
        save(&path, &s).unwrap();
        assert_eq!(load(&path).language, Language::English);
        let mut s = load(&path);
        s.language = Language::Spanish;
        save(&path, &s).unwrap();
        assert_eq!(load(&path).color_mode, ColorMode::Light);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn calendar_sources_are_shown_until_unchecked_and_remembered() {
        let dir = setup("sources");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"hidden_calendars":["future"]}"#).unwrap();
        let mut s = load(&path);
        assert!(CalendarSource::ALL.iter().all(|c| s.shows(*c)));
        s.set_shown(CalendarSource::Habits, false);
        s.set_shown(CalendarSource::Habits, false);
        save(&path, &s).unwrap();
        let mut s = load(&path);
        assert!(!s.shows(CalendarSource::Habits));
        assert!(s.shows(CalendarSource::Tasks));
        s.set_shown(CalendarSource::Habits, true);
        assert!(s.shows(CalendarSource::Habits));
        // An id this version does not know is kept.
        assert_eq!(s.hidden_calendars, ["future"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn study_preset_is_remembered_and_old_files_mean_flowtime() {
        let dir = setup("preset");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"language":"es"}"#).unwrap();
        assert_eq!(load(&path).study_preset, None);
        let s = Settings {
            study_preset: Some(crate::study::Preset::P50),
            ..Default::default()
        };
        save(&path, &s).unwrap();
        assert_eq!(load(&path).study_preset, s.study_preset);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn codes() {
        assert_eq!(Language::System.code(), None);
        assert_eq!(Language::Spanish.code(), Some("es"));
        assert_eq!(Language::English.code(), Some("en"));
    }
}
