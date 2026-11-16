// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! What the Goals view shows: the models, their refresh and the error messages.

use std::rc::Rc;

use open_productivity_core::chrono::Local;
use open_productivity_core::db::Connection;
use open_productivity_core::goals::{self, Entry, Goal, GoalError, GoalId, GoalTask};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::dates::ui_date;
use crate::tasks::ui_id;
use crate::{AppWindow, GoalEntryRow, GoalKind, GoalRow, GoalTaskRow, GoalsStore};

pub struct State {
    pub conn: Rc<Connection>,
    pub rows: Rc<VecModel<GoalRow>>,
    pub tasks: Rc<VecModel<GoalTaskRow>>,
    pub entries: Rc<VecModel<GoalEntryRow>>,
    pub window: slint::Weak<AppWindow>,
}

pub fn goal_kind(k: goals::GoalKind) -> GoalKind {
    match k {
        goals::GoalKind::Goal => GoalKind::Goal,
        goals::GoalKind::Project => GoalKind::Project,
    }
}

pub fn core_kind(k: GoalKind) -> goals::GoalKind {
    match k {
        GoalKind::Goal => goals::GoalKind::Goal,
        GoalKind::Project => goals::GoalKind::Project,
    }
}

fn goal_row(g: Goal) -> Result<GoalRow, String> {
    let is_numeric = g.numeric.is_some();
    let n = g.numeric.unwrap_or_else(|| goals::NumericProgress {
        target: 0,
        unit: String::new(),
        total: 0,
        percent: 0,
    });
    Ok(GoalRow {
        id: ui_id(g.id.0)?,
        title: g.title.into(),
        emoji: g.emoji.into(),
        description: g.description.into(),
        kind: goal_kind(g.kind),
        has_deadline: g.deadline.is_some(),
        deadline: g.deadline.map(ui_date).unwrap_or_default(),
        days_left: g.days_left.map(ui_id).transpose()?.unwrap_or_default(),
        tasks_done: ui_id(g.tasks_done.into())?,
        tasks_total: ui_id(g.tasks_total.into())?,
        tasks_percent: ui_id(g.tasks_percent.into())?,
        is_numeric,
        target: ui_id(n.target)?,
        unit: n.unit.into(),
        total: ui_id(n.total)?,
        numeric_percent: ui_id(n.percent.into())?,
    })
}

fn task_row(t: GoalTask) -> Result<GoalTaskRow, String> {
    Ok(GoalTaskRow {
        id: ui_id(t.id.0)?,
        title: t.title.into(),
        done: t.done,
        has_due: t.due.is_some(),
        due: t.due.map(ui_date).unwrap_or_default(),
    })
}

fn entry_row(e: Entry) -> Result<GoalEntryRow, String> {
    Ok(GoalEntryRow {
        id: ui_id(e.id.0)?,
        date: ui_date(e.date),
        value: ui_id(e.value)?,
    })
}

impl State {
    fn reload(&self, w: &AppWindow) -> Result<(), String> {
        let today = Local::now().date_naive();
        let rows = goals::list(&self.conn, today)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(goal_row)
            .collect::<Result<Vec<_>, _>>()?;
        let store = w.global::<GoalsStore>();
        store.set_any_goal(!rows.is_empty());
        // The open goal follows its edits; one that is gone closes the panel.
        if store.get_detail_open() {
            let id = store.get_detail().id;
            match rows.iter().find(|r| r.id == id) {
                Some(r) => {
                    store.set_detail(r.clone());
                    self.load_tasks(GoalId(id.into()))?;
                    self.load_entries(GoalId(id.into()))?;
                }
                None => store.set_detail_open(false),
            }
        }
        self.rows.set_vec(rows);
        Ok(())
    }

    fn load_tasks(&self, id: GoalId) -> Result<(), String> {
        let tasks = goals::list_tasks(&self.conn, id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(task_row)
            .collect::<Result<Vec<_>, _>>()?;
        self.tasks.set_vec(tasks);
        Ok(())
    }

    fn load_entries(&self, id: GoalId) -> Result<(), String> {
        let entries = goals::list_entries(&self.conn, id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(entry_row)
            .collect::<Result<Vec<_>, _>>()?;
        self.entries.set_vec(entries);
        Ok(())
    }

    pub fn open(&self, id: GoalId) -> Result<(), String> {
        let Some(w) = self.window.upgrade() else {
            return Ok(());
        };
        let g = goals::get(&self.conn, id, Local::now().date_naive()).map_err(|e| e.to_string())?;
        let store = w.global::<GoalsStore>();
        store.set_detail(goal_row(g)?);
        self.load_tasks(id)?;
        self.load_entries(id)?;
        store.set_detail_open(true);
        Ok(())
    }

    /// Shows a load failure in the view, or clears the last one.
    pub fn show_load(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let msg = match self.reload(&w) {
            Err(e) => w.global::<GoalsStore>().invoke_load_failed(e.into()),
            Ok(()) => Default::default(),
        };
        w.global::<GoalsStore>().set_error(msg);
    }

    pub fn message(w: &AppWindow, e: GoalError) -> SharedString {
        let s = w.global::<GoalsStore>();
        match e {
            GoalError::EmptyTitle => s.get_empty_title(),
            GoalError::NotFound => s.get_not_found(),
            GoalError::BadTarget => s.get_bad_target(),
            GoalError::BadNumber | GoalError::BadValue => s.get_bad_number(),
            GoalError::ProjectMeasure => s.get_project_measure(),
            GoalError::MeasureChanged => s.get_measure_changed(),
            GoalError::NotNumeric => s.get_not_numeric(),
            e => s.invoke_failed(e.to_string().into()),
        }
    }

    /// The message for the outcome; empty means it worked, and the view is refreshed.
    pub fn finish(&self, res: Result<(), GoalError>) -> SharedString {
        let Some(w) = self.window.upgrade() else {
            return Default::default();
        };
        match res {
            Err(e) => Self::message(&w, e),
            Ok(()) => {
                self.show_load();
                Default::default()
            }
        }
    }

    pub fn model(&self) -> ModelRc<GoalRow> {
        ModelRc::from(self.rows.clone())
    }

    pub fn tasks_model(&self) -> ModelRc<GoalTaskRow> {
        ModelRc::from(self.tasks.clone())
    }

    pub fn entries_model(&self) -> ModelRc<GoalEntryRow> {
        ModelRc::from(self.entries.clone())
    }
}
