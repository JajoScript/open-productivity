// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! What the Study view shows: the models, their refresh and the error messages.

use std::rc::Rc;

use open_productivity_core::db::Connection;
use open_productivity_core::study::{self, StudyError, Subject, SubjectFilter, SubjectId};
use slint::{ComponentHandle, SharedString, VecModel};

use crate::dates::ui_date;
use crate::tasks::ui_id;
use crate::{AppWindow, SessionRow, Strings, SubjectRow, SubjectTab};

pub struct State {
    pub conn: Rc<Connection>,
    pub rows: Rc<VecModel<SubjectRow>>,
    pub active: Rc<VecModel<SubjectRow>>,
    pub sessions: Rc<VecModel<SessionRow>>,
    pub window: slint::Weak<AppWindow>,
}

fn subject_row(s: Subject) -> Result<SubjectRow, String> {
    Ok(SubjectRow {
        id: ui_id(s.id.0)?,
        name: s.name.into(),
        duration: study::format_duration(s.total_minutes).into(),
        sessions: ui_id(s.sessions)?,
        archived: s.archived,
    })
}

fn filter(tab: SubjectTab) -> SubjectFilter {
    match tab {
        SubjectTab::Active => SubjectFilter::Active,
        SubjectTab::Archived => SubjectFilter::Archived,
        SubjectTab::All => SubjectFilter::All,
    }
}

impl State {
    fn subject_rows(&self, filter: SubjectFilter) -> Result<Vec<SubjectRow>, String> {
        study::list_subjects(&self.conn, filter)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(subject_row)
            .collect()
    }

    /// Shows one subject and its sessions in the detail.
    pub fn load_detail(&self, w: &AppWindow, id: SubjectId) -> Result<(), String> {
        let subject = study::get_subject(&self.conn, id).map_err(|e| e.to_string())?;
        let sessions = study::list_sessions(&self.conn, id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|s| {
                Ok(SessionRow {
                    id: ui_id(s.id.0)?,
                    date: ui_date(s.date),
                    duration: study::format_duration(s.minutes).into(),
                    description: s.description.into(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        w.set_subject_detail(subject_row(subject)?);
        self.sessions.set_vec(sessions);
        Ok(())
    }

    fn reload(&self, w: &AppWindow) -> Result<(), String> {
        self.rows
            .set_vec(self.subject_rows(filter(w.get_subject_tab()))?);
        self.active
            .set_vec(self.subject_rows(SubjectFilter::Active)?);
        w.set_any_subject(!self.subject_rows(SubjectFilter::All)?.is_empty());
        // The open subject stays open even if it left the current tab.
        if w.get_subject_open() {
            self.load_detail(w, SubjectId(w.get_subject_detail().id.into()))?;
        }
        Ok(())
    }

    /// Shows a load failure in the view, or clears the last one.
    pub fn show_load(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let msg = match self.reload(&w) {
            Err(e) => w.global::<Strings>().invoke_study_load_failed(e.into()),
            Ok(()) => Default::default(),
        };
        w.set_study_error(msg);
    }

    fn message(w: &AppWindow, e: StudyError) -> SharedString {
        let s = w.global::<Strings>();
        match e {
            StudyError::EmptyName => s.get_study_empty_name(),
            StudyError::BadMinutes => s.get_study_bad_minutes(),
            StudyError::FutureDate => s.get_study_future_date(),
            StudyError::SubjectNotFound => s.get_study_no_subject(),
            StudyError::SubjectArchived => s.get_study_archived(),
            StudyError::SessionNotFound => s.get_study_no_session(),
            e => s.invoke_study_failed(e.to_string().into()),
        }
    }

    /// The modal's message for the outcome; empty means it worked, and the view is refreshed.
    pub fn finish(&self, res: Result<(), StudyError>) -> SharedString {
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

    /// Like `finish` for actions without a modal: the message goes to the view.
    pub fn act(&self, res: Result<(), StudyError>) {
        let msg = self.finish(res);
        if let Some(w) = self.window.upgrade()
            && !msg.is_empty()
        {
            w.set_study_error(msg);
        }
    }
}
