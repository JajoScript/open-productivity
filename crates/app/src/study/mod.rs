// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Study view and `core::study`.

mod hours_line;
mod state;
mod timer;

use std::path::Path;
use std::rc::Rc;

use open_productivity_core::chrono::Local;
use open_productivity_core::db::Connection;
use open_productivity_core::study::{self, SessionId, SubjectId};
use open_productivity_core::tasks::{self, TaskId};
use slint::{ComponentHandle, ModelRc};

use crate::dates::core_date;
use crate::{AppWindow, Date, Strings, StudyStore};
use state::State;

/// The returned timer drives the running session's clock; the caller keeps it alive.
pub fn init(window: &AppWindow, conn: Rc<Connection>, app_dir: &Path) -> Rc<slint::Timer> {
    let state = Rc::new(State {
        conn,
        rows: Rc::default(),
        active: Rc::default(),
        sessions: Rc::default(),
        days: Rc::default(),
        topics: Rc::default(),
        today_topics: Rc::default(),
        logger_topics: Rc::default(),
        window: window.as_weak(),
        phase: Default::default(),
        settings_path: app_dir.join("settings.json"),
    });
    window
        .global::<StudyStore>()
        .set_subjects(ModelRc::from(state.rows.clone()));
    window
        .global::<StudyStore>()
        .set_active_subjects(ModelRc::from(state.active.clone()));
    window
        .global::<StudyStore>()
        .set_subject_sessions(ModelRc::from(state.sessions.clone()));
    window
        .global::<StudyStore>()
        .set_days(ModelRc::from(state.days.clone()));
    window
        .global::<StudyStore>()
        .set_topics(ModelRc::from(state.topics.clone()));
    window
        .global::<StudyStore>()
        .set_today_topics(ModelRc::from(state.today_topics.clone()));
    window
        .global::<StudyStore>()
        .set_logger_topics(ModelRc::from(state.logger_topics.clone()));
    state.show_load();

    window.global::<StudyStore>().on_create_subject({
        let state = state.clone();
        move |name| state.finish(study::create_subject(&state.conn, &name).map(|_| ()))
    });

    window.global::<StudyStore>().on_rename_subject({
        let state = state.clone();
        move |id, name| {
            state.finish(study::rename_subject(
                &state.conn,
                SubjectId(id.into()),
                &name,
            ))
        }
    });

    window.global::<StudyStore>().on_subject_tab_changed({
        let state = state.clone();
        move || state.show_load()
    });

    window.global::<StudyStore>().on_days_tab_changed({
        let state = state.clone();
        move || state.show_load()
    });

    window.global::<StudyStore>().on_hours_tab_changed({
        let state = state.clone();
        move || state.show_load()
    });

    window.global::<StudyStore>().on_open_subject({
        let state = state.clone();
        move |id| {
            let Some(w) = state.window.upgrade() else {
                return;
            };
            match state.load_detail(&w, SubjectId(id.into())) {
                Ok(()) => {
                    w.global::<StudyStore>().set_error(Default::default());
                    w.global::<StudyStore>().set_subject_open(true);
                }
                Err(e) => {
                    let msg = w.global::<Strings>().invoke_study_load_failed(e.into());
                    w.global::<StudyStore>().set_error(msg);
                }
            }
        }
    });

    window.global::<StudyStore>().on_set_subject_archived({
        let state = state.clone();
        move |id, archived| {
            state.act(study::set_subject_archived(
                &state.conn,
                SubjectId(id.into()),
                archived,
            ))
        }
    });

    window.global::<StudyStore>().on_delete_session({
        let state = state.clone();
        move |id| state.act(study::delete_session(&state.conn, SessionId(id.into())))
    });

    window.global::<StudyStore>().on_log_session({
        let state = state.clone();
        move |id, has_topic, topic, date: Date, minutes, description| {
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
                    has_topic.then(|| TaskId(topic.into())),
                    Local::now().date_naive(),
                )
            });
            state.finish(res)
        }
    });

    window.global::<StudyStore>().on_load_subject_topics({
        let state = state.clone();
        move |id| state.load_logger_topics(SubjectId(id.into()))
    });

    window.global::<StudyStore>().on_create_topic({
        let state = state.clone();
        move |id, title, has_date, date: Date| {
            let Some(w) = state.window.upgrade() else {
                return Default::default();
            };
            // A date the picker cannot produce is an error, never "no date".
            let due = match has_date.then(|| core_date(date)).transpose() {
                Ok(d) => d,
                Err(e) => return w.global::<Strings>().invoke_study_failed(e.into()),
            };
            state.finish(
                study::create_topic(&state.conn, SubjectId(id.into()), &title, due).map(|_| ()),
            )
        }
    });

    window.global::<StudyStore>().on_set_topic_done({
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
                w.global::<StudyStore>().set_error(msg);
            }
        }
    });

    timer::init(window, &state)
}
