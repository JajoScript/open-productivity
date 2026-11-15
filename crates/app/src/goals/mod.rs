// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Goals and projects: connects the view to `core::goals`.

mod state;

use std::rc::Rc;

use open_productivity_core::db::Connection;
use open_productivity_core::goals::{self, GoalDraft, GoalId};
use slint::ComponentHandle;

use self::state::{State, core_kind};
use crate::dates::core_date;
use crate::{AppWindow, Date, GoalsStore};

pub fn init(window: &AppWindow, conn: Rc<Connection>) {
    let state = Rc::new(State {
        conn,
        rows: Rc::default(),
        window: window.as_weak(),
    });
    let store = window.global::<GoalsStore>();
    store.set_goals(state.model());
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
