// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Tasks view and `core::tasks`.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use open_productivity_core::chrono::{Datelike, Local, NaiveDate};
use open_productivity_core::db::Connection;
use open_productivity_core::tasks::{self, DueLabel, TaskError, TaskId, View};
use slint::{ComponentHandle, ModelRc, Timer, TimerMode, VecModel};

use crate::{AppWindow, Date, DueKind, ListEntry, Strings, TaskRow, TaskView, lists};

pub struct State {
    pub conn: Connection,
    rows: Rc<VecModel<TaskRow>>,
    pub sidebar: Rc<VecModel<ListEntry>>,
    pub view: Cell<View>,
    pub window: slint::Weak<AppWindow>,
}

/// Slint ints are i32; a larger id is an error, not a panic.
pub fn ui_id(id: i64) -> Result<i32, String> {
    i32::try_from(id).map_err(|e| e.to_string())
}

pub fn ui_view(v: View) -> TaskView {
    match v {
        View::Today => TaskView::Today,
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

impl State {
    /// Rows of the active view plus the sidebar.
    pub fn reload(&self) -> Result<(), String> {
        let now = Local::now().fixed_offset();
        let list = tasks::list(&self.conn, self.view.get(), now).map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(list.len());
        for t in list {
            let id = ui_id(t.id.0)?;
            let kind = ui_kind(tasks::due_label(&t, now.date_naive()));
            // The date parts are always the stored due date (0 when none).
            let (day, month, year) = t
                .due
                .map_or((0, 0, 0), |d| (d.day() as i32, d.month() as i32, d.year()));
            out.push(TaskRow {
                id,
                title: t.title.into(),
                done: t.done,
                due_kind: kind,
                due_day: day,
                due_month: month,
                due_year: year,
                list_name: t.list.map(|(_, name)| name).unwrap_or_default().into(),
            });
        }
        self.rows.set_vec(out);
        self.reload_sidebar()
    }
}

/// The returned timer must be kept alive for the periodic reload to run.
pub fn init(window: &AppWindow, conn: Connection) -> Timer {
    let state = Rc::new(State {
        conn,
        rows: Rc::default(),
        sidebar: Rc::default(),
        view: Cell::new(View::Today),
        window: window.as_weak(),
    });
    window.set_tasks(ModelRc::from(state.rows.clone()));
    window.set_lists(ModelRc::from(state.sidebar.clone()));

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
        move |v| match v {
            TaskView::Today => lists::show_view(&state, View::Today, &*finish),
            TaskView::All => lists::show_view(&state, View::All, &*finish),
            // A list is selected by its id with select-list.
            TaskView::List => {}
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

    lists::init(window, &state, &finish);

    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_secs(60), move || {
        finish(state.reload())
    });
    timer
}
