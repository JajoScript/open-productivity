// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::Cell;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use open_productivity_core::db::{self, DbError};
use open_productivity_core::instances::{self, Error};
use rfd::{FileDialog, MessageButtons, MessageDialog, MessageLevel};

mod about;
mod calendar;
mod data;
mod dialogs;
mod instances_ui;
mod lists;
mod settings;
mod tasks;

slint::include_modules!();

const TITLE: &str = "Open Productivity";

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
    let app_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("open-productivity");
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
    let tasks_timer = tasks::init(&window, conn.clone());
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
mod tests {
    use super::*;
    use i_slint_backend_testing::ElementHandle;
    use slint::platform::{PointerEventButton as B, WindowEvent as E};

    fn window() -> AppWindow {
        i_slint_backend_testing::init_no_event_loop();
        let w = AppWindow::new().unwrap();
        w.window().set_size(slint::LogicalSize::new(960., 640.));
        w.show().unwrap();
        // English labels whatever the machine's language.
        slint::select_bundled_translation("").unwrap();
        w
    }

    fn find(w: &AppWindow, label: &str) -> Option<ElementHandle> {
        ElementHandle::find_by_accessible_label(w, label).next()
    }

    /// A real mouse click on the element's centre, like the user does.
    fn click(w: &AppWindow, label: &str) {
        let e = find(w, label).unwrap_or_else(|| panic!("no element {label:?}"));
        let (p, s) = (e.absolute_position(), e.size());
        let at = slint::LogicalPosition::new(p.x + s.width / 2., p.y + s.height / 2.);
        let win = w.window();
        win.dispatch_event(E::PointerMoved { position: at });
        win.dispatch_event(E::PointerPressed {
            position: at,
            button: B::Left,
        });
        win.dispatch_event(E::PointerReleased {
            position: at,
            button: B::Left,
        });
    }

    #[test]
    fn new_list_opens_the_creator_and_escape_closes_it() {
        let w = window();
        click(&w, "New list");
        let size = find(&w, "List name").unwrap().size();
        assert!(size.width > 100. && size.height >= 36., "{size:?}");
        let esc = slint::platform::Key::Escape.into();
        w.window().dispatch_event(E::KeyPressed { text: esc });
        assert!(find(&w, "List name").is_none());
    }

    #[test]
    fn rail_switches_between_tasks_and_settings() {
        let w = window();
        click(&w, "Settings");
        assert!(find(&w, "New list").is_none());
        click(&w, "Tasks");
        assert!(find(&w, "New list").is_some());
    }
}
