// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Tasks view and `core::tasks`.

use std::rc::Rc;

use open_productivity_core::db::Connection;
use open_productivity_core::tasks::{self, TaskError, TaskId};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{AppWindow, Strings, TaskRow};

fn reload(conn: &Connection, rows: &VecModel<TaskRow>) -> Result<(), String> {
    let list = tasks::list(conn).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(list.len());
    for t in list {
        // Slint ints are i32; a larger id is an error, not a panic.
        let id = i32::try_from(t.id.0).map_err(|e| e.to_string())?;
        out.push(TaskRow {
            id,
            title: t.title.into(),
            done: t.done,
        });
    }
    rows.set_vec(out);
    Ok(())
}

pub fn init(window: &AppWindow, conn: Rc<Connection>) {
    let rows = Rc::new(VecModel::<TaskRow>::default());
    window.set_tasks(ModelRc::from(rows.clone()));

    // Shows the outcome in the view; an empty title is silently ignored.
    let finish = Rc::new({
        let weak = window.as_weak();
        move |res: Result<(), String>| {
            if let Some(w) = weak.upgrade() {
                let msg = match res {
                    Ok(()) => String::new(),
                    Err(e) => w.global::<Strings>().invoke_task_failed(e.into()).into(),
                };
                w.set_tasks_error(msg.into());
            }
        }
    });

    finish(reload(&conn, &rows));

    window.on_create_task({
        let (conn, rows, finish) = (conn.clone(), rows.clone(), finish.clone());
        move |title| match tasks::create(&conn, &title) {
            Err(TaskError::EmptyTitle) => {}
            Err(e) => finish(Err(e.to_string())),
            Ok(_) => finish(reload(&conn, &rows)),
        }
    });

    window.on_set_done(
        move |id, done| match tasks::set_done(&conn, TaskId(id.into()), done) {
            Err(e) => {
                // Reload anyway so the checkbox shows what is really stored.
                let _ = reload(&conn, &rows);
                finish(Err(e.to_string()))
            }
            Ok(()) => finish(reload(&conn, &rows)),
        },
    );
}
