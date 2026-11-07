// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Tasks view and `core::tasks`.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use open_productivity_core::calendar;
use open_productivity_core::chrono::{Datelike, Local, NaiveDate};
use open_productivity_core::db::Connection;
use open_productivity_core::tasks::{self, DueLabel, Priority, Task, TaskError, TaskId, View};
use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::{
    AppWindow, Date, DueKind, KanbanColumn, ListEntry, Quadrant, Strings, TaskRow, TaskView,
    TasksTab, WeekDay, habits, kanban, lists, matrix, week,
};

pub struct State {
    pub conn: Rc<Connection>,
    rows: Rc<VecModel<TaskRow>>,
    pub panel: Rc<VecModel<ListEntry>>,
    pub view: Cell<View>,
    /// The open tab, so a reload only queries what is on screen.
    pub tab: Cell<TasksTab>,
    /// Monday of the week the Week tab shows.
    pub monday: Cell<NaiveDate>,
    pub week: Rc<VecModel<WeekDay>>,
    pub kanban: Rc<VecModel<KanbanColumn>>,
    pub matrix: Rc<VecModel<Quadrant>>,
    // Day of the last reload, so the timer only rebuilds rows when it changes.
    pub day: Cell<NaiveDate>,
    pub window: slint::Weak<AppWindow>,
}

/// Slint ints are i32; a larger id is an error, not a panic.
pub fn ui_id(id: i64) -> Result<i32, String> {
    i32::try_from(id).map_err(|e| e.to_string())
}

pub fn ui_view(v: View) -> TaskView {
    match v {
        View::Today => TaskView::Today,
        View::Scheduled => TaskView::Scheduled,
        View::Inbox => TaskView::Inbox,
        View::Overdue => TaskView::Overdue,
        View::All => TaskView::All,
        View::List(_) => TaskView::List,
    }
}

fn ui_kind(l: Option<DueLabel>) -> DueKind {
    match l {
        None => DueKind::None,
        Some(DueLabel::Overdue) => DueKind::Overdue,
        Some(DueLabel::Today) => DueKind::Today,
        Some(DueLabel::Tomorrow) => DueKind::Tomorrow,
        Some(DueLabel::On(_)) => DueKind::Date,
    }
}

pub fn task_row(t: Task, today: NaiveDate, first_done: bool) -> Result<TaskRow, String> {
    let kind = ui_kind(tasks::due_label(&t, today));
    // The date parts are always the stored due date (0 when none).
    let (day, month, year) = t
        .due
        .map_or((0, 0, 0), |d| (d.day() as i32, d.month() as i32, d.year()));
    Ok(TaskRow {
        id: ui_id(t.id.0)?,
        title: t.title.into(),
        note: t.note.into(),
        done: t.done,
        due_kind: kind,
        due_day: day,
        due_month: month,
        due_year: year,
        list_name: t.list.map(|(_, name)| name).unwrap_or_default().into(),
        urgent: t.priority.urgent,
        important: t.priority.important,
        first_done,
    })
}

/// Publishes the day the window shows; the Tasks and Habits views read it.
pub fn show_today(w: &AppWindow, today: NaiveDate) {
    w.set_today(Date {
        year: today.year(),
        month: today.month() as i32,
        day: today.day() as i32,
    });
    w.set_today_weekday(today.weekday().num_days_from_monday() as i32);
}

impl State {
    /// Only what the open tab shows: the week, or the view's rows plus the lists panel.
    pub fn reload(&self) -> Result<(), String> {
        let now = Local::now().fixed_offset();
        let today = now.date_naive();
        self.day.set(today);
        if let Some(w) = self.window.upgrade() {
            show_today(&w, today);
        }
        match self.tab.get() {
            TasksTab::Week => return self.reload_week(today),
            TasksTab::Kanban => return self.reload_kanban(today),
            TasksTab::Eisenhower => return self.reload_matrix(today),
            TasksTab::Today | TasksTab::Lists => {}
        }
        let list = tasks::list(&self.conn, self.view.get(), now).map_err(|e| e.to_string())?;
        let first_done = tasks::first_done(&list);
        let done_count = ui_id((list.len() - first_done) as i64)?;
        let rows = list
            .into_iter()
            .enumerate()
            .map(|(i, t)| task_row(t, today, i == first_done))
            .collect::<Result<Vec<_>, _>>()?;
        self.rows.set_vec(rows);
        if let Some(w) = self.window.upgrade() {
            w.set_done_count(done_count);
            if self.tab.get() == TasksTab::Today {
                habits::load_today(&w, &self.conn)?;
            }
        }
        self.reload_panel()
    }
}

/// The returned timer must be kept alive for the periodic reload to run.
pub fn init(window: &AppWindow, conn: Rc<Connection>) -> Timer {
    let state = Rc::new(State {
        conn,
        rows: Rc::default(),
        panel: Rc::default(),
        view: Cell::new(View::Today),
        tab: Cell::new(TasksTab::Today),
        monday: Cell::new(calendar::week_start(Local::now().date_naive())),
        week: Rc::default(),
        kanban: Rc::default(),
        matrix: Rc::default(),
        day: Cell::new(Local::now().date_naive()),
        window: window.as_weak(),
    });
    window.set_tasks(ModelRc::from(state.rows.clone()));
    window.set_week_days(ModelRc::from(state.week.clone()));
    window.set_kanban_columns(ModelRc::from(state.kanban.clone()));
    window.set_quadrants(ModelRc::from(state.matrix.clone()));
    window.set_lists(ModelRc::from(state.panel.clone()));

    // Shows the outcome in the view; an empty title is silently ignored.
    let finish = Rc::new({
        let weak = window.as_weak();
        move |res: Result<(), String>| {
            if let Some(w) = weak.upgrade() {
                let msg = match res {
                    Ok(()) => String::new(),
                    Err(e) => w.global::<Strings>().invoke_task_failed(e.into()).into(),
                };
                w.set_tasks_error(msg.into());
            }
        }
    });

    finish(state.reload());

    window.on_select_view({
        let (state, finish) = (state.clone(), finish.clone());
        move |v| {
            let view = match v {
                TaskView::Today => View::Today,
                TaskView::Scheduled => View::Scheduled,
                TaskView::Inbox => View::Inbox,
                TaskView::Overdue => View::Overdue,
                TaskView::All => View::All,
                // A list is selected by its id with select-list.
                TaskView::List => return,
            };
            lists::show_view(&state, view, &*finish)
        }
    });

    window.on_create_task({
        let (state, finish) = (state.clone(), finish.clone());
        move |title| {
            let today = Local::now().date_naive();
            match tasks::create(&state.conn, &title, state.view.get(), today) {
                Err(TaskError::EmptyTitle) => {}
                Err(e) => finish(Err(e.to_string())),
                Ok(_) => finish(state.reload()),
            }
        }
    });

    // Reload even on failure so the view shows what is really stored.
    let after = {
        let (state, finish) = (state.clone(), finish.clone());
        move |res: Result<(), TaskError>| {
            let reloaded = state.reload();
            finish(res.map_err(|e| e.to_string()).and(reloaded));
        }
    };

    window.on_set_done({
        let (state, after) = (state.clone(), after.clone());
        move |id, done| after(tasks::set_done(&state.conn, TaskId(id.into()), done))
    });

    window.on_set_due({
        let (state, after, finish) = (state.clone(), after.clone(), finish.clone());
        move |id, d: Date| {
            // A date the picker cannot produce is an error, never "clear the date".
            match NaiveDate::from_ymd_opt(d.year, d.month as u32, d.day as u32) {
                Some(due) => after(tasks::set_due_date(
                    &state.conn,
                    TaskId(id.into()),
                    Some(due),
                )),
                None => finish(Err(format!("{}-{}-{}", d.year, d.month, d.day))),
            }
        }
    });

    window.on_clear_due({
        let (state, after) = (state.clone(), after.clone());
        move |id| after(tasks::set_due_date(&state.conn, TaskId(id.into()), None))
    });

    window.on_set_list({
        let (state, after) = (state.clone(), after.clone());
        move |id, list| {
            after(tasks::set_list(
                &state.conn,
                TaskId(id.into()),
                lists::list_arg(list),
            ))
        }
    });

    window.on_update_task({
        let (state, after) = (state.clone(), after.clone());
        move |id, title, note, urgent, important| {
            let priority = Priority { urgent, important };
            match tasks::update(&state.conn, TaskId(id.into()), &title, &note, priority) {
                // The editor stays open with the typed text and shows the message itself.
                Err(TaskError::EmptyTitle) => false,
                res => {
                    after(res);
                    true
                }
            }
        }
    });

    window.on_delete_task({
        let (state, after) = (state.clone(), after.clone());
        move |id| after(tasks::delete(&state.conn, TaskId(id.into())))
    });

    window.on_select_tab({
        let (state, finish) = (state.clone(), finish.clone());
        move |tab| {
            match tab {
                TasksTab::Today => state.view.set(View::Today),
                // The week tab always opens on the current week.
                TasksTab::Week => state
                    .monday
                    .set(calendar::week_start(Local::now().date_naive())),
                TasksTab::Kanban | TasksTab::Eisenhower | TasksTab::Lists => {}
            }
            state.tab.set(tab);
            finish(state.reload())
        }
    });

    lists::init(window, &state, &finish);
    week::init(window, &state, &finish);
    kanban::init(window, &state, &finish);
    matrix::init(window, &state, &finish);

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(60), move || {
        // set_vec closes open popups, so reload only when the day rolls over.
        if Local::now().date_naive() != state.day.get() {
            finish(state.reload())
        }
    });
    timer
}
