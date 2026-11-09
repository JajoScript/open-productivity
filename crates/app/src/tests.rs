// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Tests for the app wiring.

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
    assert!(find(&w, "Add task").is_none());
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
fn calendar_module_shows_the_month_and_navigates() {
    use slint::Model;

    let (dir, w, _timer) = window_with_task("calendar", "a");
    click(&w, "Calendar");
    assert!(find(&w, "Next month").is_some());
    assert_eq!(w.get_calendar_weeks().row_count(), 6);
    let month = w.get_calendar_month();
    click(&w, "Next month");
    let next = w.get_calendar_month();
    assert_ne!((month.year, month.month), (next.year, next.month));
    click(&w, "Today");
    let back = w.get_calendar_month();
    assert_eq!((back.year, back.month), (month.year, month.month));
    drop(w);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn kanban_tab_shows_the_board_alone() {
    let w = window();
    click(&w, "Kanban");
    assert!(find(&w, "Add task").is_none());
    assert!(find(&w, "Next week").is_none());
    click(&w, "Today");
    assert!(find(&w, "Add task").is_some() || find(&w, "Nothing due today.").is_some());
}

#[test]
fn add_task_opens_the_creator_which_rejects_an_empty_title() {
    let (dir, w, _timer) = window_with_task("creator", "a");
    click(&w, "Add task");
    // Inside a popup positions are popup-relative, so a pointer click would land outside it.
    find(&w, "Create")
        .unwrap()
        .invoke_accessible_default_action();
    assert!(find(&w, "Task title").is_some(), "stays open with no title");
    assert!(find(&w, "Write a title for the task.").is_some());
    let esc = slint::platform::Key::Escape.into();
    w.window().dispatch_event(E::KeyPressed { text: esc });
    assert!(find(&w, "Task title").is_none());
    drop(w);
    let _ = fs::remove_dir_all(&dir);
}

/// A window wired to a fresh database holding one undated task named `title`.
fn window_with_task(name: &str, title: &str) -> (PathBuf, AppWindow, slint::Timer) {
    use open_productivity_core::chrono::Local;
    use open_productivity_core::tasks::{self as core_tasks, View};

    let dir = std::env::temp_dir().join(format!("op-app-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let conn = db::open(&dir.join("test.db")).unwrap();
    core_tasks::create(&conn, title, View::All, Local::now().date_naive()).unwrap();
    let w = window();
    let timer = tasks::init(&w, Rc::new(conn), &dir);
    (dir, w, timer)
}

/// Presses on the card whose checkbox is labelled `card`, past its title, then drags it
/// in 10 steps to 120px below the element labelled `target`, calling `step` after each.
fn drag(w: &AppWindow, card: &str, target: &str, mut step: impl FnMut(slint::LogicalPosition)) {
    let c = find(w, card).unwrap().absolute_position();
    let from = slint::LogicalPosition::new(c.x + 60., c.y + 16.);
    let t = find(w, target).unwrap().absolute_position();
    let to = slint::LogicalPosition::new(t.x + 40., t.y + 120.);
    let win = w.window();
    win.dispatch_event(E::PointerMoved { position: from });
    win.dispatch_event(E::PointerPressed {
        position: from,
        button: B::Left,
    });
    for i in 1..=10 {
        let f = i as f32 / 10.;
        let position =
            slint::LogicalPosition::new(from.x + (to.x - from.x) * f, from.y + (to.y - from.y) * f);
        win.dispatch_event(E::PointerMoved { position });
        step(position);
    }
    win.dispatch_event(E::PointerReleased {
        position: to,
        button: B::Left,
    });
}

#[test]
fn dragging_a_card_moves_it_to_another_column() {
    use slint::Model;

    let (dir, w, _timer) = window_with_task("drag", "drag me");
    click(&w, "Kanban");
    let in_column = |i: usize| {
        w.get_kanban_columns()
            .row_data(i)
            .unwrap()
            .tasks
            .row_count()
    };
    assert_eq!((in_column(0), in_column(1)), (1, 0));

    let ghost = || ElementHandle::find_by_element_id(&w, "DragBoard::ghost").next();
    // The floating copy holds the cursor at the point where the card was grabbed.
    drag(&w, "drag me", "In progress", |p| {
        let g = ghost().expect("no floating copy while dragging");
        let (gp, gs) = (g.absolute_position(), g.size());
        assert!(
            p.x >= gp.x && p.x <= gp.x + gs.width && p.y >= gp.y && p.y <= gp.y + gs.height,
            "cursor {p:?} outside the copy at {gp:?} {gs:?}"
        );
    });

    assert_eq!((in_column(0), in_column(1)), (0, 1));
    assert!(ghost().is_none(), "the floating copy outlived the drop");
    drop(w);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn dragging_a_card_onto_a_quadrant_sets_its_marks() {
    use slint::Model;

    let (dir, w, _timer) = window_with_task("matrix", "sort me");
    click(&w, "Eisenhower");
    let in_quadrant = |i: usize| w.get_quadrants().row_data(i).unwrap().tasks.row_count();
    // A new task is neither urgent nor important.
    assert_eq!((in_quadrant(0), in_quadrant(3)), (0, 1));

    drag(&w, "sort me", "Do first", |_| {});

    assert_eq!((in_quadrant(0), in_quadrant(3)), (1, 0));
    let t = w
        .get_quadrants()
        .row_data(0)
        .unwrap()
        .tasks
        .row_data(0)
        .unwrap();
    assert!(t.urgent && t.important);
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
