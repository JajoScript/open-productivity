// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! What the Goals view shows: the models, their refresh and the error messages.

use std::rc::Rc;

use open_productivity_core::chrono::Local;
use open_productivity_core::db::Connection;
use open_productivity_core::goals::{self, Goal, GoalError, GoalId};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::dates::ui_date;
use crate::tasks::ui_id;
use crate::{AppWindow, GoalKind, GoalRow, GoalsStore};

pub struct State {
    pub conn: Rc<Connection>,
    pub rows: Rc<VecModel<GoalRow>>,
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
    Ok(GoalRow {
        id: ui_id(g.id.0)?,
        title: g.title.into(),
        emoji: g.emoji.into(),
        description: g.description.into(),
        kind: goal_kind(g.kind),
        has_deadline: g.deadline.is_some(),
        deadline: g.deadline.map(ui_date).unwrap_or_default(),
        days_left: g.days_left.map(ui_id).transpose()?.unwrap_or_default(),
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
                Some(r) => store.set_detail(r.clone()),
                None => store.set_detail_open(false),
            }
        }
        self.rows.set_vec(rows);
        Ok(())
    }

    pub fn open(&self, id: GoalId) -> Result<(), String> {
        let Some(w) = self.window.upgrade() else {
            return Ok(());
        };
        let g = goals::get(&self.conn, id, Local::now().date_naive()).map_err(|e| e.to_string())?;
        let store = w.global::<GoalsStore>();
        store.set_detail(goal_row(g)?);
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

    fn message(w: &AppWindow, e: GoalError) -> SharedString {
        let s = w.global::<GoalsStore>();
        match e {
            GoalError::EmptyTitle => s.get_empty_title(),
            GoalError::NotFound => s.get_not_found(),
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
}
