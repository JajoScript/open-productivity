// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Goals and projects: connects the view to `core::goals`.

mod state;

use std::rc::Rc;

use open_productivity_core::chrono::Local;
use open_productivity_core::db::Connection;
use open_productivity_core::goals::{self, EntryId, GoalDraft, GoalError, GoalId, Numeric};
use open_productivity_core::tasks::{self, TaskId};
use slint::ComponentHandle;

use self::state::{State, core_kind};
use crate::dates::core_date;
use crate::{AppWindow, Date, GoalsStore, Strings};

pub fn init(window: &AppWindow, conn: Rc<Connection>) {
    let state = Rc::new(State {
        conn,
        rows: Rc::default(),
        tasks: Rc::default(),
        entries: Rc::default(),
        window: window.as_weak(),
    });
    let store = window.global::<GoalsStore>();
    store.set_goals(state.model());
    store.set_tasks(state.tasks_model());
    store.set_entries(state.entries_model());
    state.show_load();

    // A date the picker cannot produce is an error, never "no deadline". The measure is read as
    // numbers only when the goal has one.
    fn draft(
        title: &str,
        emoji: &str,
        description: &str,
        has_deadline: bool,
        deadline: Date,
        measure: Option<(&str, &str)>,
    ) -> Result<GoalDraft, GoalError> {
        Ok(GoalDraft {
            title: title.into(),
            emoji: emoji.into(),
            description: description.into(),
            deadline: has_deadline
                .then(|| core_date(deadline))
                .transpose()
                .map_err(GoalError::BadDate)?,
            numeric: measure
                .map(|(target, unit)| {
                    Ok::<_, GoalError>(Numeric {
                        target: goals::parse_count(target)?,
                        unit: unit.into(),
                    })
                })
                .transpose()?,
        })
    }

    store.on_create({
        let state = state.clone();
        move |kind, title, emoji, description, has_deadline, deadline, numeric, target, unit| {
            let measure = numeric.then_some((target.as_str(), unit.as_str()));
            state.finish(
                draft(
                    &title,
                    &emoji,
                    &description,
                    has_deadline,
                    deadline,
                    measure,
                )
                .and_then(|d| goals::create(&state.conn, core_kind(kind), &d))
                .map(|_| ()),
            )
        }
    });

    store.on_update({
        let state = state.clone();
        move |id, title, emoji, description, has_deadline, deadline, numeric, target, unit| {
            let measure = numeric.then_some((target.as_str(), unit.as_str()));
            state.finish(
                draft(
                    &title,
                    &emoji,
                    &description,
                    has_deadline,
                    deadline,
                    measure,
                )
                .and_then(|d| goals::update(&state.conn, GoalId(id.into()), &d)),
            )
        }
    });

    store.on_delete({
        let state = state.clone();
        // The reload closes the panel of the goal that is gone.
        move |id| state.finish(goals::delete(&state.conn, GoalId(id.into())))
    });

    store.on_open({
        let state = state.clone();
        move |id| {
            if let Err(e) = state.open(GoalId(id.into()))
                && let Some(w) = state.window.upgrade()
            {
                let s = w.global::<GoalsStore>();
                s.set_error(s.invoke_load_failed(e.into()));
            }
        }
    });

    store.on_create_task({
        let state = state.clone();
        move |goal, title, has_due, due: Date| {
            // A date the picker cannot produce is an error, never "no date".
            match has_due.then(|| core_date(due)).transpose() {
                Ok(due) => state.finish(
                    goals::create_task(&state.conn, GoalId(goal.into()), &title, due).map(|_| ()),
                ),
                Err(e) => state_failed(&state, e),
            }
        }
    });

    store.on_add_entry({
        let state = state.clone();
        move |goal, date, value| {
            state.finish(
                core_date(date)
                    .map_err(GoalError::BadDate)
                    .and_then(|date| {
                        let value = goals::parse_count(&value)?;
                        goals::add_entry(&state.conn, GoalId(goal.into()), date, value)
                    })
                    .map(|_| ()),
            )
        }
    });

    store.on_delete_entry({
        let state = state.clone();
        move |id| {
            let res = goals::delete_entry(&state.conn, EntryId(id.into()));
            state.show_load();
            if let (Err(e), Some(w)) = (res, state.window.upgrade()) {
                w.global::<GoalsStore>().set_error(State::message(&w, e));
            }
        }
    });

    store.on_set_task_done({
        let state = state.clone();
        move |id, done| {
            let res = tasks::set_done(
                &state.conn,
                TaskId(id.into()),
                done,
                Local::now().date_naive(),
            );
            state.show_load();
            if let (Err(e), Some(w)) = (res, state.window.upgrade()) {
                let msg = crate::tasks::task_message(&w.global::<Strings>(), e);
                w.global::<GoalsStore>().set_error(msg);
            }
        }
    });

    store.on_refresh({
        let state = state.clone();
        move || state.show_load()
    });
}

fn state_failed(state: &State, e: String) -> slint::SharedString {
    match state.window.upgrade() {
        Some(w) => w.global::<GoalsStore>().invoke_failed(e.into()),
        None => Default::default(),
    }
}
