// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue for the Data section: exporting a copy of the open database.

use std::rc::Rc;

use open_productivity_core::chrono::Local;
use open_productivity_core::db::{self, Connection};
use rfd::FileDialog;
use slint::ComponentHandle;

use crate::{AppWindow, Strings};

pub fn init(window: &AppWindow, conn: Rc<Connection>) {
    let weak = window.as_weak();
    window.on_export_copy(move || {
        let Some(w) = weak.upgrade() else {
            return;
        };
        w.set_export_status("".into());
        let name = format!("open-productivity-{}.db", Local::now().format("%Y-%m-%d"));
        let Some(dest) = FileDialog::new()
            .set_title(w.global::<Strings>().get_export_title().as_str())
            .add_filter("SQLite", &["db"])
            .set_file_name(name)
            .save_file()
        else {
            return;
        };
        let s = w.global::<Strings>();
        match db::export(&conn, &dest) {
            Ok(()) => {
                w.set_export_failed(false);
                w.set_export_status(s.invoke_export_done(dest.display().to_string().into()));
            }
            Err(e) => {
                w.set_export_failed(true);
                w.set_export_status(s.invoke_export_failed(e.to_string().into()));
            }
        }
    });
}
