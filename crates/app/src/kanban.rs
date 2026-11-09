// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Kanban tab and `core::tasks::kanban`.

use std::rc::Rc;

use open_productivity_core::chrono::{NaiveDate, Utc};
use open_productivity_core::tasks::{self, TaskError, TaskId};
use slint::{DataTransfer, ModelRc, SharedString, VecModel};

use crate::tasks::{State, task_row};
use crate::{AppWindow, KanbanColumn, Stage};

impl State {
    pub fn reload_kanban(&self, today: NaiveDate) -> Result<(), String> {
        let columns = tasks::kanban(&self.conn, Utc::now()).map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(columns.len());
        for (stage, list) in [Stage::Icebox, Stage::InProgress, Stage::Done]
            .into_iter()
            .zip(columns)
        {
            let rows = list
                .into_iter()
                .map(|t| task_row(t, today, false))
                .collect::<Result<Vec<_>, _>>()?;
            out.push(KanbanColumn {
                stage,
                tasks: ModelRc::new(VecModel::from(rows)),
            });
        }
        self.kanban.set_vec(out);
        Ok(())
    }
}

/// Marks a drag payload as one of our task cards, so text dragged from elsewhere is refused.
const PAYLOAD: &str = "open-productivity/task:";

fn task_payload(id: i32) -> DataTransfer {
    SharedString::from(format!("{PAYLOAD}{id}")).into()
}

/// The task id a drag payload carries; 0 when it is not one of our cards (ids start at 1).
fn payload_task(data: &DataTransfer) -> i32 {
    let text = data.plain_text().unwrap_or_default();
    text.strip_prefix(PAYLOAD)
        .and_then(|id| id.parse().ok())
        .filter(|id| *id > 0)
        .unwrap_or(0)
}

pub fn init(
    window: &AppWindow,
    state: &Rc<State>,
    after: impl Fn(Result<(), TaskError>) + 'static,
) {
    window.on_task_payload(task_payload);
    window.on_payload_task(|data| payload_task(&data));

    window.on_move_task({
        let state = state.clone();
        move |id, stage| {
            let stage = match stage {
                Stage::Icebox => tasks::Stage::Icebox,
                Stage::InProgress => tasks::Stage::InProgress,
                Stage::Done => tasks::Stage::Done,
            };
            let today = open_productivity_core::chrono::Local::now().date_naive();
            after(tasks::set_stage(
                &state.conn,
                TaskId(id.into()),
                stage,
                today,
            ));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_round_trips_and_refuses_foreign_text() {
        assert_eq!(payload_task(&task_payload(42)), 42);
        for foreign in [
            "",
            "42",
            "hello",
            "open-productivity/task:",
            "open-productivity/task:-3",
        ] {
            let data: DataTransfer = SharedString::from(foreign).into();
            assert_eq!(payload_task(&data), 0, "{foreign:?}");
        }
    }
}
