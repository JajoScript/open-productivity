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
mod kanban;
mod lists;
mod settings;
mod tasks;
mod week;

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
        click(&w, "Lists");
        click(&w, "New list");
        let size = find(&w, "List name").unwrap().size();
        assert!(size.width > 100. && size.height >= 36., "{size:?}");
        let esc = slint::platform::Key::Escape.into();
        w.window().dispatch_event(E::KeyPressed { text: esc });
        assert!(find(&w, "List name").is_none());
    }

    #[test]
    fn week_tab_replaces_the_lists_and_tasks() {
        let w = window();
        click(&w, "Week");
        assert!(find(&w, "Next week").is_some());
        assert!(find(&w, "Add task…").is_none());
        click(&w, "Lists");
        assert!(find(&w, "Next week").is_none());
        assert!(find(&w, "New list").is_some());
        let s = w.global::<Strings>();
        assert_eq!(
            s.invoke_week_range(30, 11, 6, 12),
            "November 30 – December 6"
        );
    }

    #[test]
    fn kanban_tab_shows_the_board_alone() {
        let w = window();
        click(&w, "Kanban");
        assert!(find(&w, "Add task…").is_none());
        assert!(find(&w, "Next week").is_none());
        click(&w, "Today");
        assert!(find(&w, "Add task…").is_some() || find(&w, "Nothing due today.").is_some());
    }

    #[test]
    fn dragging_a_card_moves_it_to_another_column() {
        use open_productivity_core::chrono::Local;
        use open_productivity_core::tasks::{self as core_tasks, View};
        use slint::Model;

        let dir = std::env::temp_dir().join(format!("op-app-drag-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let conn = db::open(&dir.join("test.db")).unwrap();
        core_tasks::create(&conn, "drag me", View::All, Local::now().date_naive()).unwrap();
        let w = window();
        let _timer = tasks::init(&w, Rc::new(conn));
        click(&w, "Kanban");

        let in_column = |i: usize| {
            w.get_kanban_columns()
                .row_data(i)
                .unwrap()
                .tasks
                .row_count()
        };
        assert_eq!((in_column(0), in_column(1)), (1, 0));

        // Press on the card's title, then drag in steps onto the In progress column.
        let check = find(&w, "drag me").unwrap();
        let c = check.absolute_position();
        let from = slint::LogicalPosition::new(c.x + 60., c.y + 16.);
        let column = find(&w, "In progress").unwrap().absolute_position();
        let to = slint::LogicalPosition::new(column.x + 40., column.y + 120.);
        let win = w.window();
        win.dispatch_event(E::PointerMoved { position: from });
        win.dispatch_event(E::PointerPressed {
            position: from,
            button: B::Left,
        });
        let ghost = || ElementHandle::find_by_element_id(&w, "KanbanView::ghost").next();
        // The floating copy holds the cursor at the point where the card was grabbed.
        let under_cursor = |p: slint::LogicalPosition| {
            let g = ghost().expect("no floating copy while dragging");
            let (gp, gs) = (g.absolute_position(), g.size());
            assert!(
                p.x >= gp.x && p.x <= gp.x + gs.width && p.y >= gp.y && p.y <= gp.y + gs.height,
                "cursor {p:?} outside the copy at {gp:?} {gs:?}"
            );
        };
        for step in 1..=10 {
            let f = step as f32 / 10.;
            let position = slint::LogicalPosition::new(
                from.x + (to.x - from.x) * f,
                from.y + (to.y - from.y) * f,
            );
            win.dispatch_event(E::PointerMoved { position });
            if step == 5 || step == 10 {
                under_cursor(position);
            }
        }
        win.dispatch_event(E::PointerReleased {
            position: to,
            button: B::Left,
        });

        assert_eq!((in_column(0), in_column(1)), (0, 1));
        assert!(ghost().is_none(), "the floating copy outlived the drop");
        drop(w);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn long_date_names_the_weekday_month_and_day() {
        let w = window();
        let s = w.global::<Strings>();
        // 0 is Monday, so 3 is Thursday.
        assert_eq!(s.invoke_long_date(5, 11, 3), "Thursday, November 5");
    }

    #[test]
    fn rail_switches_modules_and_the_tasks_tab_survives() {
        let w = window();
        // The lists panel only shows in the Lists tab.
        assert!(find(&w, "New list").is_none());
        click(&w, "Lists");
        // The smart list cards share the panel's width evenly, whatever their names.
        let widths: Vec<_> = ["Today", "Scheduled", "Inbox", "Overdue"]
            .iter()
            .map(|l| {
                ElementHandle::find_by_accessible_label(&w, l)
                    .find(|e| e.size().height == 64.)
                    .unwrap()
                    .size()
                    .width
            })
            .collect();
        assert!(widths.iter().all(|x| *x == widths[0]), "{widths:?}");
        click(&w, "Settings");
        assert!(find(&w, "Inbox").is_none());
        click(&w, "Tasks");
        assert!(find(&w, "New list").is_some());
    }
}
