// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use open_productivity_core::db::{self, DbError};
use open_productivity_core::instances::{self, Error};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageLevel};

slint::include_modules!();

/// User-facing strings, kept together so they can move to translations later.
mod messages {
    use std::fmt::Display;
    use std::path::Path;

    pub const TITLE: &str = "Open Productivity";
    pub const ALREADY_RUNNING: &str = "Open Productivity ya está abierta.";
    pub const PICK_FOLDER: &str = "Elige la carpeta donde guardar tus datos";
    pub const NEWER_SCHEMA: &str = "Esta instancia fue creada con una versión más nueva de Open Productivity. Actualiza la aplicación para abrirla.";
    pub const MISSING_ARG: &str = "Falta la carpeta después de --instance.";
    pub const CORRUPT_REGISTRY: &str =
        "El archivo con la lista de instancias está dañado y se reemplazará al elegir una carpeta.";

    pub fn folder_not_found(p: &Path) -> String {
        format!(
            "No se encontró la carpeta de la instancia: {}. Elige otra.",
            p.display()
        )
    }

    pub fn db_failed(e: &dyn Display) -> String {
        format!("No se pudo abrir la base de datos: {e}")
    }

    pub fn failed(e: &dyn Display) -> String {
        format!("Error inesperado: {e}")
    }
}

fn log(app_dir: &Path, level: &str, msg: &str) {
    // shortcut: unix timestamps, switch to readable dates once chrono is a dependency
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // A failing log must not hide the dialog, so write errors are ignored.
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(app_dir.join("open-productivity.log"))
    {
        let _ = writeln!(f, "{secs} {level} {msg}");
    }
}

fn dialog(level: MessageLevel, text: &str) {
    MessageDialog::new()
        .set_level(level)
        .set_title(messages::TITLE)
        .set_description(text)
        .set_buttons(MessageButtons::Ok)
        .show();
}

fn fatal(app_dir: &Path, msg: &str) -> ! {
    log(app_dir, "ERROR", msg);
    dialog(MessageLevel::Error, msg);
    std::process::exit(1)
}

fn warn(app_dir: &Path, msg: &str) {
    log(app_dir, "WARN", msg);
    dialog(MessageLevel::Warning, msg);
}

/// Asks for a folder and remembers it; `None` when the user cancels.
fn pick_and_register(app_dir: &Path, registry: &Path) -> Option<PathBuf> {
    let dir = FileDialog::new()
        .set_title(messages::PICK_FOLDER)
        .pick_folder()?;
    if let Err(e) = instances::register(registry, &dir) {
        fatal(app_dir, &messages::failed(&e));
    }
    Some(dir)
}

fn choose_instance(app_dir: &Path) -> Option<PathBuf> {
    let mut args = std::env::args_os().skip(1);
    while let Some(a) = args.next() {
        if a == "--instance" {
            // Explicit instance: leave the registry untouched.
            return match args.next() {
                Some(p) => Some(PathBuf::from(p)),
                None => fatal(app_dir, messages::MISSING_ARG),
            };
        }
    }

    let registry = app_dir.join("instances.json");
    match instances::resolve(&registry) {
        Ok(dir) => return Some(dir),
        Err(Error::Missing(None)) => {}
        Err(Error::Missing(Some(p))) => warn(app_dir, &messages::folder_not_found(&p)),
        Err(Error::Corrupt(_)) => warn(app_dir, messages::CORRUPT_REGISTRY),
        Err(e @ Error::Io(_)) => fatal(app_dir, &messages::failed(&e)),
    }
    pick_and_register(app_dir, &registry)
}

fn main() {
    let app_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("open-productivity");
    if let Err(e) = fs::create_dir_all(&app_dir) {
        fatal(&app_dir, &messages::failed(&e));
    }

    // Held until exit; the OS releases the lock if the process dies.
    let lock = match File::create(app_dir.join("open-productivity.lock")) {
        Ok(f) => f,
        Err(e) => fatal(&app_dir, &messages::failed(&e)),
    };
    // shortcut: a second launch only shows a dialog, upgrade to focusing the running window
    // when there is an IPC channel (e.g. when file/URL opening is added).
    if lock.try_lock().is_err() {
        dialog(MessageLevel::Info, messages::ALREADY_RUNNING);
        return;
    }

    let Some(instance) = choose_instance(&app_dir) else {
        return;
    };

    if let Err(e) = fs::create_dir_all(&instance) {
        fatal(&app_dir, &messages::failed(&e));
    }
    // Kept alive while the window runs; it will be handed to the UI later.
    let _conn = match db::open(&instance.join("open-productivity.db")) {
        Ok(c) => c,
        Err(DbError::NewerSchema { .. }) => fatal(&app_dir, messages::NEWER_SCHEMA),
        Err(e @ DbError::Sqlite(_)) => fatal(&app_dir, &messages::db_failed(&e)),
    };

    let window = match AppWindow::new() {
        Ok(w) => w,
        Err(e) => fatal(&app_dir, &messages::failed(&e)),
    };
    let name = instance.file_name().map_or_else(
        || instance.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    window.set_instance_name(name.into());
    if let Err(e) = window.run() {
        fatal(&app_dir, &messages::failed(&e));
    }
    drop(lock);
}
