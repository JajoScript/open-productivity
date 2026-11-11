// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! What the Study view shows: the models, their refresh and the error messages.

use std::rc::Rc;

use open_productivity_core::chrono::Local;
use open_productivity_core::db::Connection;
use open_productivity_core::study::{self, StudyError, Subject, SubjectFilter, SubjectId};
use slint::{ComponentHandle, SharedString, VecModel};

use crate::dates::ui_date;
use crate::tasks::ui_id;
use crate::{AppWindow, SessionRow, Strings, SubjectRow, SubjectTab, TopicRow};

pub struct State {
    pub conn: Rc<Connection>,
    pub rows: Rc<VecModel<SubjectRow>>,
    pub active: Rc<VecModel<SubjectRow>>,
    pub sessions: Rc<VecModel<SessionRow>>,
    pub topics: Rc<VecModel<TopicRow>>,
    pub today_topics: Rc<VecModel<TopicRow>>,
    pub logger_topics: Rc<VecModel<TopicRow>>,
    pub window: slint::Weak<AppWindow>,
    /// The Pomodoro phase seen at the last refresh, to tell when it changed.
    pub phase: std::cell::Cell<Option<study::Phase>>,
    pub settings_path: std::path::PathBuf,
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

fn topic_row(t: study::Topic) -> Result<TopicRow, String> {
    Ok(TopicRow {
        id: ui_id(t.id.0)?,
        title: t.title.into(),
        subject: t.subject_name.into(),
        has_due: t.due.is_some(),
        due: t.due.map(ui_date).unwrap_or_default(),
        done: t.done,
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
                    topic: s.topic.unwrap_or_default().into(),
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
        self.topics.set_vec(
            study::list_topics(&self.conn, Local::now().fixed_offset())
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(topic_row)
                .collect::<Result<Vec<_>, _>>()?,
        );
        self.today_topics.set_vec(
            study::today_topics(&self.conn, Local::now().fixed_offset())
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(topic_row)
                .collect::<Result<Vec<_>, _>>()?,
        );
        w.set_any_subject(!self.subject_rows(SubjectFilter::All)?.is_empty());
        // The open subject stays open even if it left the current tab.
        if w.get_subject_open() {
            self.load_detail(w, SubjectId(w.get_subject_detail().id.into()))?;
        }
        Ok(())
    }

    /// Fills the session modal's topic choices with the pending topics of `subject`.
    pub fn load_logger_topics(&self, subject: SubjectId) {
        let topics = study::subject_topics(&self.conn, subject)
            .map_err(|e| e.to_string())
            .and_then(|t| t.into_iter().map(topic_row).collect::<Result<Vec<_>, _>>());
        match topics {
            Ok(t) => self.logger_topics.set_vec(t),
            Err(e) => {
                self.logger_topics.set_vec(Vec::new());
                if let Some(w) = self.window.upgrade() {
                    let msg = w.global::<Strings>().invoke_study_load_failed(e.into());
                    w.set_study_error(msg);
                }
            }
        }
    }

    /// Shows a load failure in the view, or clears the last one.
    pub fn show_load(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        w.set_study_notice(Default::default());
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
            StudyError::EmptyTitle => s.get_study_empty_title(),
            StudyError::BadMinutes => s.get_study_bad_minutes(),
            StudyError::FutureDate => s.get_study_future_date(),
            StudyError::SubjectNotFound => s.get_study_no_subject(),
            StudyError::SubjectArchived => s.get_study_archived(),
            StudyError::TopicNotFound => s.get_study_no_topic(),
            StudyError::SessionNotFound => s.get_study_no_session(),
            StudyError::SessionActive => s.get_study_session_active(),
            StudyError::NoActiveSession => s.get_study_no_active_session(),
            StudyError::TooShort => s.get_study_too_short(),
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
