// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    AppWindow::new()?.run()
}
