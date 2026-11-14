// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the settings view and `core::settings`.

use std::path::Path;
use std::rc::Rc;

use open_productivity_core::settings::{
    self, ColorMode as CoreColorMode, Language as CoreLanguage,
};
use slint::ComponentHandle;
use slint::winit_030::{WinitWindowAccessor, winit::window::Theme};

use crate::{AppWindow, ColorMode, FinanceStore, Language, Strings};

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

fn mode_to_ui(m: CoreColorMode) -> ColorMode {
    match m {
        CoreColorMode::System => ColorMode::System,
        CoreColorMode::Light => ColorMode::Light,
        CoreColorMode::Dark => ColorMode::Dark,
    }
}

fn mode_from_ui(m: ColorMode) -> CoreColorMode {
    match m {
        ColorMode::System => CoreColorMode::System,
        ColorMode::Light => CoreColorMode::Light,
        ColorMode::Dark => CoreColorMode::Dark,
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
            Ok(()) => {
                crate::finances::set_style_for(c);
                return;
            }
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

// The native title bar follows Windows, not Palette, so it is told the chosen mode.
fn set_title_bar(window: &slint::Window, mode: CoreColorMode) {
    let theme = match mode {
        CoreColorMode::System => None,
        CoreColorMode::Light => Some(Theme::Light),
        CoreColorMode::Dark => Some(Theme::Dark),
    };
    // No-op when the backend is not winit or the window does not exist yet.
    window.with_winit_window(|w| w.set_theme(theme));
}

pub fn init(window: &AppWindow, app_dir: &Path) {
    let path = app_dir.join("settings.json");
    let loaded = settings::load(&path);
    apply(loaded.language, app_dir);
    window.set_current_language(to_ui(loaded.language));
    // Set before the window is shown, so it never flashes the wrong colors.
    window.set_current_color_mode(mode_to_ui(loaded.color_mode));
    // winit only creates its window once the event loop runs, so wait for it.
    let (weak, mode) = (window.as_weak(), loaded.color_mode);
    let _ = slint::spawn_local(async move {
        let Some(w) = weak.upgrade() else { return };
        if w.window().winit_window().await.is_ok() {
            set_title_bar(w.window(), mode);
        }
    });

    // Each change reloads the file and edits only its own field, so it keeps the other.
    let report = {
        let weak = window.as_weak();
        Rc::new(move |saved: Result<(), std::io::Error>| {
            let Some(w) = weak.upgrade() else { return };
            // Applied for this session even if it could not be saved.
            w.set_settings_error(match saved {
                Ok(()) => "".into(),
                Err(e) => w
                    .global::<Strings>()
                    .invoke_settings_failed(e.to_string().into()),
            });
        })
    };

    let (weak, p, r) = (window.as_weak(), path.clone(), report.clone());
    let app_dir = app_dir.to_path_buf();
    window.on_set_language(move |l| {
        let language = from_ui(l);
        let mut current = settings::load(&p);
        current.language = language;
        let saved = settings::save(&p, &current);
        apply(language, &app_dir);
        if let Some(w) = weak.upgrade() {
            w.set_current_language(l);
            // Amounts follow the language's separators.
            w.global::<FinanceStore>().invoke_refresh();
        }
        r(saved);
    });

    let weak = window.as_weak();
    window.on_set_color_mode(move |m| {
        let mut current = settings::load(&path);
        current.color_mode = mode_from_ui(m);
        let mode = current.color_mode;
        let saved = settings::save(&path, &current);
        if let Some(w) = weak.upgrade() {
            w.set_current_color_mode(m);
            set_title_bar(w.window(), mode);
        }
        report(saved);
    });
}
