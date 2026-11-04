// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue for the About section: version, license text and the source-code link.

use std::process::Command;

use slint::ComponentHandle;

use crate::{AppWindow, Strings};

pub fn init(window: &AppWindow) {
    window.set_version(env!("CARGO_PKG_VERSION").into());
    window.set_license_text(include_str!("../../../LICENSE").into());

    let weak = window.as_weak();
    window.on_open_source(move || {
        // explorer exits non-zero even when it opens the link, so only a spawn error counts.
        let result = Command::new("explorer")
            .arg(env!("CARGO_PKG_REPOSITORY"))
            .spawn();
        let Some(w) = weak.upgrade() else { return };
        w.set_settings_error(match result {
            Ok(_) => "".into(),
            Err(e) => w
                .global::<Strings>()
                .invoke_open_failed(e.to_string().into()),
        });
    });
}
