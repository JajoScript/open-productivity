// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Eisenhower tab and `core::tasks::eisenhower`.

use std::rc::Rc;

use open_productivity_core::chrono::NaiveDate;
use open_productivity_core::tasks::{self, Priority, TaskError, TaskId};
use slint::{ModelRc, VecModel};

use crate::tasks::{State, task_row};
use crate::{AppWindow, Quadrant};

impl State {
    pub fn reload_matrix(&self, today: NaiveDate) -> Result<(), String> {
        let quadrants = tasks::eisenhower(&self.conn).map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(quadrants.len());
        for (priority, list) in Priority::QUADRANTS.into_iter().zip(quadrants) {
            let rows = list
                .into_iter()
                .map(|t| task_row(t, today, false))
                .collect::<Result<Vec<_>, _>>()?;
            out.push(Quadrant {
                urgent: priority.urgent,
                important: priority.important,
                tasks: ModelRc::new(VecModel::from(rows)),
            });
        }
        self.matrix.set_vec(out);
        Ok(())
    }
}

pub fn init(
    window: &AppWindow,
    state: &Rc<State>,
    after: impl Fn(Result<(), TaskError>) + 'static,
) {
    window.on_set_priority({
        let state = state.clone();
        move |id, urgent, important| {
            let priority = Priority { urgent, important };
            after(tasks::set_priority(
                &state.conn,
                TaskId(id.into()),
                priority,
            ));
        }
    });
}
