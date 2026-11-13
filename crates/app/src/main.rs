// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::Cell;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use open_productivity_core::chrono::Local;
use open_productivity_core::db::{self, DbError};
use open_productivity_core::instances::{self, Error};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageLevel};

mod about;
mod calendar;
mod calendar_view;
mod data;
mod dates;
mod dialogs;
mod habits;
mod instances_ui;
mod kanban;
mod lists;
mod matrix;
mod settings;
mod study;
mod tasks;
mod week;

slint::include_modules!();

const TITLE: &str = "Open Productivity";

fn log(app_dir: &Path, level: &str, msg: &str) {
    let now = Local::now().format("%Y-%m-%d %H:%M:%S");
    // A failing log must not hide the dialog, so write errors are ignored.
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(app_dir.join("open-productivity.log"))
    {
        let _ = writeln!(f, "{now} {level} {msg}");
    }
}

fn dialog(level: MessageLevel, text: &str) {
    MessageDialog::new()
        .set_level(level)
        .set_title(TITLE)
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
fn pick_and_register(s: &Strings, app_dir: &Path, registry: &Path) -> Option<PathBuf> {
    let dir = FileDialog::new()
        .set_title(s.get_pick_folder().as_str())
        .pick_folder()?;
    if let Err(e) = instances::register(registry, &dir) {
        fatal(app_dir, &s.invoke_failed(e.to_string().into()));
    }
    Some(dir)
}

fn choose_instance(s: &Strings, app_dir: &Path) -> Option<PathBuf> {
    let mut args = std::env::args_os().skip(1);
    while let Some(a) = args.next() {
        if a == "--instance" {
            // Explicit instance: leave the registry untouched.
            let Some(p) = args.next() else {
                fatal(app_dir, &s.get_missing_arg())
            };
            let dir = PathBuf::from(p);
            return dialogs::confirm_new(s, &dir).then_some(dir);
        }
    }

    let registry = app_dir.join("instances.json");
    match instances::resolve(&registry) {
        Ok(dir) if dialogs::confirm_new(s, &dir) => return Some(dir),
        Ok(_) => {}
        Err(Error::Missing(None)) => {}
        Err(Error::Missing(Some(p))) => warn(
            app_dir,
            &s.invoke_folder_not_found(p.display().to_string().into()),
        ),
        Err(Error::Corrupt(_)) => warn(app_dir, &s.get_corrupt_registry()),
        Err(e @ Error::Io(_)) => fatal(app_dir, &s.invoke_failed(e.to_string().into())),
    }
    pick_and_register(s, app_dir, &registry)
}

fn main() {
    // QA points this elsewhere so testing never touches the real settings, registry or lock.
    let app_dir = std::env::var_os("OPEN_PRODUCTIVITY_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("open-productivity")
        });
    // A release build has no console: without this a panic would leave no trace at all.
    let (log_dir, default_hook) = (app_dir.clone(), std::panic::take_hook());
    std::panic::set_hook(Box::new(move |info| {
        log(&log_dir, "PANIC", &info.to_string());
        default_hook(info);
    }));
    // The strings live in the window, so a failure here has nothing to translate with yet.
    let window = match AppWindow::new() {
        Ok(w) => w,
        Err(e) => fatal(&app_dir, &format!("Could not start Open Productivity: {e}")),
    };
    // Before any dialog, so they are translated too.
    settings::init(&window, &app_dir);
    about::init(&window);
    let s = window.global::<Strings>();
    if let Err(e) = fs::create_dir_all(&app_dir) {
        fatal(&app_dir, &s.invoke_failed(e.to_string().into()));
    }

    // Held until exit; the OS releases the lock if the process dies.
    let lock = match File::create(app_dir.join("open-productivity.lock")) {
        Ok(f) => f,
        Err(e) => fatal(&app_dir, &s.invoke_failed(e.to_string().into())),
    };
    // shortcut: a second launch only shows a dialog, upgrade to focusing the running window
    // when there is an IPC channel (e.g. when file/URL opening is added).
    if lock.try_lock().is_err() {
        dialog(MessageLevel::Info, &s.get_already_running());
        return;
    }

    let Some(instance) = choose_instance(&s, &app_dir) else {
        return;
    };

    if let Err(e) = fs::create_dir_all(&instance) {
        fatal(&app_dir, &s.invoke_failed(e.to_string().into()));
    }
    let conn = match db::open(&instance.join(db::FILE_NAME)) {
        Ok(c) => c,
        Err(DbError::NewerSchema { .. }) => fatal(&app_dir, &s.get_newer_schema()),
        Err(e) => fatal(&app_dir, &s.invoke_db_failed(e.to_string().into())),
    };
    calendar::init(&window);
    let switching = Rc::new(Cell::new(false));
    instances_ui::init(&window, &app_dir, &instance, &switching);
    // Shared by the modules; the window's callbacks hold the only references.
    let conn = Rc::new(conn);
    let tasks_timer = tasks::init(&window, conn.clone(), &app_dir);
    let habits_timer = habits::init(&window, conn.clone());
    let study_timer = study::init(&window, conn.clone(), &app_dir);
    data::init(&window, conn);

    let name = instance.file_name().map_or_else(
        || instance.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    window.set_instance_name(name.into());
    if let Err(e) = window.run() {
        fatal(&app_dir, &s.invoke_failed(e.to_string().into()));
    }
    if switching.get() {
        // Release the database and the lock before the new process needs them. The window
        // owns the callbacks that hold the connection, so it goes too.
        drop(tasks_timer);
        drop(habits_timer);
        drop(study_timer);
        drop(window);
        drop(lock);
        let spawned =
            std::env::current_exe().and_then(|exe| std::process::Command::new(exe).spawn());
        if let Err(e) = spawned {
            // The window (and its translations) is gone, so this message stays in English.
            fatal(
                &app_dir,
                &format!("Could not restart Open Productivity: {e}"),
            );
        }
    }
}

#[cfg(test)]
mod tests;
