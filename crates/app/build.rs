// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

use slint_build::{CompilerConfiguration, DefaultTranslationContext};

fn main() {
    // No context keeps the .po files free of `msgctxt`.
    let config = CompilerConfiguration::new()
        .with_bundled_translations("lang")
        .with_default_translation_context(DefaultTranslationContext::None)
        // UI tests find elements through this; release builds stay without it.
        .with_debug_info(std::env::var("PROFILE").as_deref() == Ok("debug"));
    slint_build::compile_with_config("ui/app-window.slint", config)
        .expect("failed to compile Slint UI");

    // Embeds the .ico as the exe icon (Explorer, taskbar).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("ui/app-icons/app.ico")
            .compile()
            .expect("failed to embed Windows icon");
    }
}
