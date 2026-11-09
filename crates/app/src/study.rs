// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Study view and `core::study`.

use std::rc::Rc;

use open_productivity_core::chrono::Local;
use open_productivity_core::db::Connection;
use open_productivity_core::study::{self, StudyError, SubjectId};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::dates::core_date;
use crate::tasks::ui_id;
use crate::{AppWindow, Date, Strings, SubjectRow};

struct State {
    conn: Rc<Connection>,
    rows: Rc<VecModel<SubjectRow>>,
    window: slint::Weak<AppWindow>,
}

impl State {
    fn reload(&self) -> Result<(), String> {
        let rows = study::list_subjects(&self.conn)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|s| {
                Ok(SubjectRow {
                    id: ui_id(s.id.0)?,
                    name: s.name.into(),
                    duration: study::format_duration(s.total_minutes).into(),
                    sessions: ui_id(s.sessions)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.rows.set_vec(rows);
        Ok(())
    }

    /// Shows a load failure in the view, or clears the last one.
    fn show_load(&self) {
        let res = self.reload();
        if let Some(w) = self.window.upgrade() {
            let msg = match res {
                Err(e) => w.global::<Strings>().invoke_study_load_failed(e.into()),
                Ok(()) => Default::default(),
            };
            w.set_study_error(msg);
        }
    }

    /// The modal's message for the outcome; empty means it worked, and the list is refreshed.
    fn finish(&self, res: Result<(), StudyError>) -> SharedString {
        let Some(w) = self.window.upgrade() else {
            return Default::default();
        };
        let s = w.global::<Strings>();
        match res {
            Err(StudyError::EmptyName) => s.get_study_empty_name(),
            Err(StudyError::BadMinutes) => s.get_study_bad_minutes(),
            Err(StudyError::FutureDate) => s.get_study_future_date(),
            Err(StudyError::SubjectNotFound) => s.get_study_no_subject(),
            Err(e) => s.invoke_study_failed(e.to_string().into()),
            Ok(()) => {
                self.show_load();
                Default::default()
            }
        }
    }
}

pub fn init(window: &AppWindow, conn: Rc<Connection>) {
    let state = Rc::new(State {
        conn,
        rows: Rc::default(),
        window: window.as_weak(),
    });
    window.set_subjects(ModelRc::from(state.rows.clone()));
    state.show_load();

    window.on_create_subject({
        let state = state.clone();
        move |name| state.finish(study::create_subject(&state.conn, &name).map(|_| ()))
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
