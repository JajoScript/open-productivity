// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

use slint_build::{CompilerConfiguration, DefaultTranslationContext};

fn main() {
    // No context keeps the .po files free of `msgctxt`.
    let config = CompilerConfiguration::new()
        .with_bundled_translations("lang")
        .with_default_translation_context(DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app-window.slint", config)
        .expect("failed to compile Slint UI");
}
