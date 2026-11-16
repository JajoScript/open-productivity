// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Goals and projects: connects the view to `core::goals`.

mod state;

use std::rc::Rc;

use open_productivity_core::chrono::Local;
use open_productivity_core::db::Connection;
use open_productivity_core::goals::{self, GoalDraft, GoalId};
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
        window: window.as_weak(),
    });
    let store = window.global::<GoalsStore>();
    store.set_goals(state.model());
    store.set_tasks(state.tasks_model());
    state.show_load();

    // A date the picker cannot produce is an error, never "no deadline".
    fn draft(
        title: &str,
        emoji: &str,
        description: &str,
        has_deadline: bool,
        deadline: Date,
    ) -> Result<GoalDraft, String> {
        Ok(GoalDraft {
            title: title.into(),
            emoji: emoji.into(),
            description: description.into(),
            deadline: has_deadline.then(|| core_date(deadline)).transpose()?,
        })
    }

    store.on_create({
        let state = state.clone();
        move |kind, title, emoji, description, has_deadline, deadline| match draft(
            &title,
            &emoji,
            &description,
            has_deadline,
            deadline,
        ) {
            Ok(d) => state.finish(goals::create(&state.conn, core_kind(kind), &d).map(|_| ())),
            Err(e) => state_failed(&state, e),
        }
    });

    store.on_update({
        let state = state.clone();
        move |id, title, emoji, description, has_deadline, deadline| match draft(
            &title,
            &emoji,
            &description,
            has_deadline,
            deadline,
        ) {
            Ok(d) => state.finish(goals::update(&state.conn, GoalId(id.into()), &d)),
            Err(e) => state_failed(&state, e),
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
