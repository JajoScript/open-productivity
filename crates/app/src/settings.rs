// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the settings view and `core::settings`.

use std::path::Path;

use open_productivity_core::settings::{self, Language as CoreLanguage, Settings};
use slint::ComponentHandle;

use crate::{AppWindow, Language, Strings};

fn to_ui(l: CoreLanguage) -> Language {
    match l {
        CoreLanguage::System => Language::System,
        CoreLanguage::Spanish => Language::Spanish,
        CoreLanguage::English => Language::English,
    }
}

fn from_ui(l: Language) -> CoreLanguage {
    match l {
        Language::System => CoreLanguage::System,
        Language::Spanish => CoreLanguage::Spanish,
        Language::English => CoreLanguage::English,
    }
}

/// Selects the bundled translation; "" is the English source text.
fn apply(language: CoreLanguage, app_dir: &Path) {
    let candidates: Vec<String> = match language.code() {
        Some(code) => vec![code.to_owned()],
        None => {
            // Same rule Slint uses at startup: full locale, then its base ("es-CL" -> "es").
            let locale = sys_locale::get_locale().unwrap_or_default();
            let base = locale
                .split(['-', '_', '@'])
                .next()
                .unwrap_or("")
                .to_owned();
            vec![locale, base, String::new()]
        }
    };
    let mut last_error = None;
    for c in &candidates {
        match slint::select_bundled_translation(c) {
            Ok(()) => return,
            Err(e) => last_error = Some(e),
        }
    }
    if let Some(e) = last_error {
        crate::log(
            app_dir,
            "WARN",
            &format!("could not select the language: {e}"),
        );
    }
}

pub fn init(window: &AppWindow, app_dir: &Path) {
    let path = app_dir.join("settings.json");
    let language = settings::load(&path).language;
    apply(language, app_dir);
    window.set_current_language(to_ui(language));

    let weak = window.as_weak();
    let app_dir = app_dir.to_path_buf();
    window.on_set_language(move |l| {
        let language = from_ui(l);
        let saved = settings::save(&path, &Settings { language });
        apply(language, &app_dir);
        let Some(w) = weak.upgrade() else { return };
        w.set_current_language(l);
        // Applied for this session even if it could not be saved.
        w.set_settings_error(match saved {
            Ok(()) => "".into(),
            Err(e) => w
                .global::<Strings>()
                .invoke_settings_failed(e.to_string().into()),
        });
    });
}
