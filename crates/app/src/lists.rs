// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the sidebar and `core::lists`.

use std::rc::Rc;

use open_productivity_core::chrono::Local;
use open_productivity_core::lists::{self, ListError, ListId};
use open_productivity_core::tasks::{self, View};
use slint::ComponentHandle;

use crate::tasks::{State, ui_id, ui_view};
use crate::{AppWindow, ListEntry, Strings};

/// The Slint side has no optional int, so 0 stands for "no list" (SQLite rowids start at 1).
pub fn list_arg(id: i32) -> Option<ListId> {
    (id != 0).then(|| ListId(id.into()))
}

pub fn show_view(state: &State, view: View, finish: &dyn Fn(Result<(), String>)) {
    state.view.set(view);
    finish(state.reload());
}

impl State {
    pub fn reload_sidebar(&self) -> Result<(), String> {
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
        self.sidebar.set_vec(entries);
        if let Some(w) = self.window.upgrade() {
            w.set_today_count(ui_id(counts.today.into())?);
            w.set_all_count(ui_id(counts.all.into())?);
            w.set_current_list_name(current_name.into());
            w.set_current_list_id(match view {
                View::List(id) => ui_id(id.0)?,
                _ => 0,
            });
            w.set_current_view(ui_view(view));
        }
        Ok(())
    }

    pub fn show_error(&self, msg: impl FnOnce(&Strings<'_>) -> slint::SharedString) {
        if let Some(w) = self.window.upgrade() {
            w.set_tasks_error(msg(&w.global::<Strings>()));
        }
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

    window.on_create_list({
        let (state, finish) = (state.clone(), finish.clone());
        move |name| match lists::create(&state.conn, &name) {
            Err(ListError::EmptyName) => {}
            Err(ListError::DuplicateName) => state.show_error(|s| s.get_list_exists()),
            Err(e) => state.show_error(|s| s.invoke_list_failed(e.to_string().into())),
            Ok(_) => finish(state.reload()),
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
            finish(res.map_err(|e| e.to_string()).and(state.reload()));
        }
    });
}
