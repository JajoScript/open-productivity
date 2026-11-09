// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Study view and `core::study`.

mod state;

use std::rc::Rc;

use open_productivity_core::chrono::Local;
use open_productivity_core::db::Connection;
use open_productivity_core::study::{self, SessionId, SubjectId};
use slint::{ComponentHandle, ModelRc};

use crate::dates::core_date;
use crate::{AppWindow, Date, Strings};
use state::State;

pub fn init(window: &AppWindow, conn: Rc<Connection>) {
    let state = Rc::new(State {
        conn,
        rows: Rc::default(),
        active: Rc::default(),
        sessions: Rc::default(),
        window: window.as_weak(),
    });
    window.set_subjects(ModelRc::from(state.rows.clone()));
    window.set_active_subjects(ModelRc::from(state.active.clone()));
    window.set_subject_sessions(ModelRc::from(state.sessions.clone()));
    state.show_load();

    window.on_create_subject({
        let state = state.clone();
        move |name| state.finish(study::create_subject(&state.conn, &name).map(|_| ()))
    });

    window.on_rename_subject({
        let state = state.clone();
        move |id, name| {
            state.finish(study::rename_subject(
                &state.conn,
                SubjectId(id.into()),
                &name,
            ))
        }
    });

    window.on_subject_tab_changed({
        let state = state.clone();
        move || state.show_load()
    });

    window.on_open_subject({
        let state = state.clone();
        move |id| {
            let Some(w) = state.window.upgrade() else {
                return;
            };
            match state.load_detail(&w, SubjectId(id.into())) {
                Ok(()) => {
                    w.set_study_error(Default::default());
                    w.set_subject_open(true);
                }
                Err(e) => {
                    let msg = w.global::<Strings>().invoke_study_load_failed(e.into());
                    w.set_study_error(msg);
                }
            }
        }
    });

    window.on_set_subject_archived({
        let state = state.clone();
        move |id, archived| {
            state.act(study::set_subject_archived(
                &state.conn,
                SubjectId(id.into()),
                archived,
            ))
        }
    });

    window.on_delete_study_session({
        let state = state.clone();
        move |id| state.act(study::delete_session(&state.conn, SessionId(id.into())))
    });

    window.on_log_study_session({
        let state = state.clone();
        move |id, date: Date, minutes, description| {
            let Some(w) = state.window.upgrade() else {
                return Default::default();
            };
            // A date the picker cannot produce is an error, never "today".
            let date = match core_date(date) {
                Ok(d) => d,
                Err(e) => return w.global::<Strings>().invoke_study_failed(e.into()),
            };
            let res = study::parse_minutes(&minutes).and_then(|m| {
                study::log_manual_session(
                    &state.conn,
                    SubjectId(id.into()),
                    date,
                    m,
                    &description,
                    Local::now().date_naive(),
                )
            });
            state.finish(res)
        }
    });
}
