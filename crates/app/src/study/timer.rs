// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The session clock on screen: starts, finishes and discards the running session.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use open_productivity_core::chrono::Local;
use open_productivity_core::settings;
use open_productivity_core::study::{self, Phase, Preset, SubjectId};
use open_productivity_core::tasks::TaskId;
use slint::{ComponentHandle, Timer, TimerMode};

use super::state::State;
use crate::{Strings, StudyMode, StudyStore};

/// Ticks between two saves of the clock.
const HEARTBEAT_TICKS: u32 = 60;

fn preset_of(mode: StudyMode) -> Option<Preset> {
    match mode {
        StudyMode::Flowtime => None,
        StudyMode::Pomodoro25 => Some(Preset::P25),
        StudyMode::Pomodoro50 => Some(Preset::P50),
        StudyMode::Pomodoro15 => Some(Preset::P15),
    }
}

fn mode_of(preset: Option<Preset>) -> StudyMode {
    match preset {
        None => StudyMode::Flowtime,
        Some(Preset::P25) => StudyMode::Pomodoro25,
        Some(Preset::P50) => StudyMode::Pomodoro50,
        Some(Preset::P15) => StudyMode::Pomodoro15,
    }
}

impl State {
    /// Shows the running session, or hides the card. Returns whether one is running.
    fn show_active(&self) -> bool {
        let Some(w) = self.window.upgrade() else {
            return false;
        };
        match study::active_session(&self.conn, Local::now().fixed_offset()) {
            Ok(Some(a)) => {
                w.global::<StudyStore>().set_active(true);
                w.global::<StudyStore>()
                    .set_active_subject(a.subject_name.into());
                w.global::<StudyStore>()
                    .set_active_topic(a.topic.unwrap_or_default().into());
                // A Pomodoro counts its phase down; Flowtime counts up.
                let secs = a.pomodoro.as_ref().map_or(a.elapsed_secs, |p| p.left_secs);
                w.global::<StudyStore>()
                    .set_clock(study::format_clock(secs).into());
                w.global::<StudyStore>().set_paused(a.paused);
                w.global::<StudyStore>().set_pomodoro(a.pomodoro.is_some());
                w.global::<StudyStore>()
                    .set_on_break(a.pomodoro.as_ref().is_some_and(|p| p.phase == Phase::Break));
                w.global::<StudyStore>()
                    .set_pomodoros(a.pomodoro.as_ref().map_or(0, |p| p.pomodoros) as i32);
                self.announce_phase(&w, a.pomodoro.map(|p| p.phase));
                true
            }
            Ok(None) => {
                w.global::<StudyStore>().set_active(false);
                self.phase.set(None);
                false
            }
            Err(e) => {
                w.global::<StudyStore>().set_error(
                    w.global::<Strings>()
                        .invoke_study_load_failed(e.to_string().into()),
                );
                false
            }
        }
    }
}

impl State {
    /// Tells the user when the phase changed since the last refresh.
    fn announce_phase(&self, w: &crate::AppWindow, now: Option<Phase>) {
        let before = self.phase.replace(now);
        if before.is_some() && before != now {
            let s = w.global::<Strings>();
            w.global::<StudyStore>().set_notice(match now {
                Some(Phase::Break) => s.invoke_study_break_started(),
                _ => s.invoke_study_focus_started(),
            });
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

    window
        .global::<StudyStore>()
        .set_mode(mode_of(settings::load(&state.settings_path).study_preset));
    if state.show_active() {
        run();
    }

    window.global::<StudyStore>().on_preview(|mode| {
        let secs = preset_of(mode).map_or(0, |p| p.focus_minutes() * 60);
        study::format_clock(secs).into()
    });
    window
        .global::<StudyStore>()
        .on_break_minutes(|mode| preset_of(mode).map_or(0, |p| p.break_minutes() as i32));

    window.global::<StudyStore>().on_start_session({
        let (state, run) = (state.clone(), run);
        move |id, has_topic, topic, mode| {
            let res = study::start_session(
                &state.conn,
                SubjectId(id.into()),
                has_topic.then(|| TaskId(topic.into())),
                Local::now().fixed_offset(),
                preset_of(mode),
            );
            let ok = res.is_ok();
            // The next session offers this mode again.
            let saved = ok.then(|| {
                let mut current = settings::load(&state.settings_path);
                current.study_preset = preset_of(mode);
                settings::save(&state.settings_path, &current)
            });
            let msg = state.finish(res);
            if ok && state.show_active() {
                run();
            }
            // After the reload, which clears the error line; the session runs anyway.
            if let (Some(Err(e)), Some(w)) = (saved, state.window.upgrade()) {
                let msg = w
                    .global::<Strings>()
                    .invoke_settings_failed(e.to_string().into());
                w.global::<StudyStore>().set_error(msg);
            }
            msg
        }
    });

    window.global::<StudyStore>().on_finish_session({
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
                    w.global::<StudyStore>().set_notice(s.invoke_study_saved(
                        study::format_duration(minutes).into(),
                        study::format_duration(study::suggested_break(minutes)).into(),
                    ));
                }
                // The session keeps running; only the message changes.
                Err(e) => state.act(Err(e)),
            }
        }
    });

    window.global::<StudyStore>().on_discard_session({
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

    window.global::<StudyStore>().on_pause_session({
        let state = state.clone();
        move || {
            state.act(study::pause_session(
                &state.conn,
                Local::now().fixed_offset(),
            ));
            state.show_active();
        }
    });

    window.global::<StudyStore>().on_resume_session({
        let state = state.clone();
        move || {
            state.act(study::resume_session(
                &state.conn,
                Local::now().fixed_offset(),
            ));
            state.show_active();
        }
    });

    timer
}
