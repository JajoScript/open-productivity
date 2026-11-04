// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Yes/No questions shared by startup and the instance switcher.

use std::path::Path;

use open_productivity_core::instances;
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

use crate::{Strings, TITLE};

fn ask(text: &str) -> bool {
    MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(TITLE)
        .set_description(text)
        .set_buttons(MessageButtons::YesNo)
        .show()
        == MessageDialogResult::Yes
}

/// Only a folder the user just picked may become an instance without asking,
/// so a typo or a moved database never turns into a silent empty instance.
pub fn confirm_new(s: &Strings, dir: &Path) -> bool {
    instances::has_database(dir)
        || ask(s
            .invoke_no_instance_here(dir.display().to_string().into())
            .as_str())
}

/// True when the folder is fine to use: not synced, or the user accepted the risk.
pub fn confirm_not_synced(s: &Strings, dir: &Path) -> bool {
    !instances::looks_synced(dir)
        || ask(s
            .invoke_synced_warning(dir.display().to_string().into())
            .as_str())
}
