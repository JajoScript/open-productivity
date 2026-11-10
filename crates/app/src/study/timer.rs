// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The Flowtime clock on screen: starts, finishes and discards the running session.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use open_productivity_core::chrono::Local;
use open_productivity_core::study::{self, SubjectId};
use open_productivity_core::tasks::TaskId;
use slint::{ComponentHandle, Timer, TimerMode};

use super::state::State;
use crate::Strings;

/// Ticks between two saves of the clock.
const HEARTBEAT_TICKS: u32 = 60;

impl State {
    /// Shows the running session, or hides the card. Returns whether one is running.
    fn show_active(&self) -> bool {
        let Some(w) = self.window.upgrade() else {
            return false;
        };
        match study::active_session(&self.conn, Local::now().fixed_offset()) {
            Ok(Some(a)) => {
                w.set_study_active(true);
                w.set_study_active_subject(a.subject_name.into());
                w.set_study_active_topic(a.topic.unwrap_or_default().into());
                w.set_study_clock(study::format_clock(a.elapsed_secs).into());
                true
            }
            Ok(None) => {
                w.set_study_active(false);
                false
            }
            Err(e) => {
                w.set_study_error(
                    w.global::<Strings>()
                        .invoke_study_load_failed(e.to_string().into()),
                );
                false
            }
        }
    }
}

/// Wires the session callbacks. The timer ticks every second only while a session runs.
pub fn init(window: &crate::AppWindow, state: &Rc<State>) -> Rc<Timer> {
    let timer = Rc::new(Timer::default());
    let ticks = Rc::new(Cell::new(0));
    let run = {
        let (state, timer, ticks) = (state.clone(), timer.clone(), ticks.clone());
        Rc::new(move || {
            ticks.set(0);
            // Bank the time since the last save now (e.g. a restart), before the gap outgrows
            // MAX_GAP_SECS while waiting for the first heartbeat.
            if let Err(e) = study::heartbeat(&state.conn, Local::now().fixed_offset()) {
                state.act(Err(e));
            }
            let (state, timer, ticks) = (state.clone(), timer.clone(), ticks.clone());
            let tick_timer = Rc::downgrade(&timer);
            timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
                ticks.set(ticks.get() + 1);
                if ticks.get() % HEARTBEAT_TICKS == 0
                    && let Err(e) = study::heartbeat(&state.conn, Local::now().fixed_offset())
                {
                    state.act(Err(e));
                }
                if !state.show_active()
                    && let Some(t) = tick_timer.upgrade()
                {
                    t.stop();
                }
            });
        })
    };

    if state.show_active() {
        run();
    }

    window.on_start_study({
        let (state, run) = (state.clone(), run);
        move |id, has_topic, topic| {
            let res = study::start_session(
                &state.conn,
                SubjectId(id.into()),
                has_topic.then(|| TaskId(topic.into())),
                Local::now().fixed_offset(),
            );
            let ok = res.is_ok();
            let msg = state.finish(res);
            if ok && state.show_active() {
                run();
            }
            msg
        }
    });

    window.on_finish_study({
        let (state, timer) = (state.clone(), timer.clone());
        move || {
            let res = study::finish_session(&state.conn, Local::now().fixed_offset());
            let Some(w) = state.window.upgrade() else {
                return;
            };
            match res {
                Ok(minutes) => {
                    timer.stop();
                    state.show_active();
                    state.show_load();
                    let s = w.global::<Strings>();
                    w.set_study_notice(s.invoke_study_saved(
                        study::format_duration(minutes).into(),
                        study::format_duration(study::suggested_break(minutes)).into(),
                    ));
                }
                // The session keeps running; only the message changes.
                Err(e) => state.act(Err(e)),
            }
        }
    });

    window.on_discard_study({
        let (state, timer) = (state.clone(), timer.clone());
        move || {
            let res = study::discard_session(&state.conn);
            if res.is_ok() {
                timer.stop();
            }
            state.act(res);
            state.show_active();
        }
    });

    timer
}
