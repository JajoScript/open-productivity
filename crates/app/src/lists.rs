// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the lists panel and `core::lists`.

use std::rc::Rc;

use open_productivity_core::chrono::Local;
use open_productivity_core::lists::{self, ListError, ListId};
use open_productivity_core::tasks::{self, View};
use slint::{ComponentHandle, SharedString};

use crate::tasks::{State, ui_id, ui_view};
use crate::{AppWindow, ListEntry, Strings, ViewCounts};

/// The Slint side has no optional int, so 0 stands for "no list" (SQLite rowids start at 1).
pub fn list_arg(id: i32) -> Option<ListId> {
    (id != 0).then(|| ListId(id.into()))
}

/// The user-facing text of a list error; only database errors show raw.
pub fn list_message(s: &Strings, e: ListError) -> SharedString {
    match e {
        ListError::EmptyName => s.get_list_name_empty(),
        ListError::DuplicateName => s.get_list_exists(),
        ListError::NotFound => s.get_list_not_found(),
        e @ ListError::Sqlite(_) => s.invoke_list_failed(e.to_string().into()),
    }
}

pub fn show_view(state: &State, view: View, finish: &dyn Fn(Result<(), String>)) {
    state.view.set(view);
    finish(state.reload());
}

impl State {
    pub fn reload_panel(&self) -> Result<(), String> {
        let all = lists::all(&self.conn).map_err(|e| e.to_string())?;
        let counts =
            tasks::counts(&self.conn, Local::now().fixed_offset()).map_err(|e| e.to_string())?;
        let view = self.view.get();
        let mut current_name = String::new();
        let mut entries = Vec::with_capacity(all.len());
        for l in all {
            if view == View::List(l.id) {
                current_name.clone_from(&l.name);
            }
            entries.push(ListEntry {
                id: ui_id(l.id.0)?,
                count: ui_id(counts.for_list(l.id).into())?,
                name: l.name.into(),
            });
        }
        self.panel.set_vec(entries);
        if let Some(w) = self.window.upgrade() {
            w.set_counts(ViewCounts {
                today: ui_id(counts.today.into())?,
                scheduled: ui_id(counts.scheduled.into())?,
                inbox: ui_id(counts.inbox.into())?,
                overdue: ui_id(counts.overdue.into())?,
                all: ui_id(counts.all.into())?,
            });
            w.set_can_create(view.can_create());
            w.set_pending_count(ui_id(counts.for_view(view).into())?);
            w.set_current_list_name(current_name.into());
            w.set_current_list_id(match view {
                View::List(id) => ui_id(id.0)?,
                _ => 0,
            });
            w.set_current_view(ui_view(view));
        }
        Ok(())
    }
}

pub fn init(
    window: &AppWindow,
    state: &Rc<State>,
    finish: &Rc<impl Fn(Result<(), String>) + 'static>,
) {
    window.on_select_list({
        let (state, finish) = (state.clone(), finish.clone());
        move |id| show_view(&state, View::List(ListId(id.into())), &*finish)
    });

    // Returns the message to show in the list creator; empty closes it.
    window.on_create_list({
        let (state, finish) = (state.clone(), finish.clone());
        move |name| {
            let Some(w) = state.window.upgrade() else {
                return Default::default();
            };
            let s = w.global::<Strings>();
            match lists::create(&state.conn, &name) {
                Err(e) => list_message(&s, e),
                Ok(_) => {
                    finish(state.reload());
                    Default::default()
                }
            }
        }
    });

    window.on_delete_list({
        let (state, finish) = (state.clone(), finish.clone());
        move |id| {
            let id = ListId(id.into());
            let res = lists::delete(&state.conn, id);
            // Whatever the outcome, the list is gone or never was: leave its view.
            if state.view.get() == View::List(id) {
                state.view.set(View::Today);
            }
            let reloaded = state.reload();
            match (res, state.window.upgrade()) {
                (Err(e), Some(w)) => state.show_message(list_message(&w.global::<Strings>(), e)),
                (Err(_), None) => {}
                (Ok(()), _) => finish(reloaded),
            }
        }
    });
}
