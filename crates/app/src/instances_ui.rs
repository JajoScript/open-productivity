// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue for the instance switcher: lists, creates, opens and unlists instances.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use open_productivity_core::db;
use open_productivity_core::instances;
use rfd::FileDialog;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{AppWindow, InstanceRow, Strings, dialogs};

struct Ctx {
    window: slint::Weak<AppWindow>,
    registry: PathBuf,
    current: PathBuf,
    /// Set when the app must restart into another instance once the loop ends.
    switching: Rc<Cell<bool>>,
}

impl Ctx {
    fn fail(&self, w: &AppWindow, e: impl ToString) {
        w.set_instances_status(
            w.global::<Strings>()
                .invoke_instances_failed(e.to_string().into()),
        );
    }

    fn refresh(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        match instances::list(&self.registry) {
            Ok(list) => {
                let rows: Vec<InstanceRow> = list
                    .into_iter()
                    .map(|i| InstanceRow {
                        name: i
                            .path
                            .file_name()
                            .map_or_else(
                                || i.path.display().to_string(),
                                |n| n.to_string_lossy().into_owned(),
                            )
                            .into(),
                        current: i.path == self.current,
                        exists: i.exists,
                        path: i.path.display().to_string().into(),
                    })
                    .collect();
                w.set_instances(ModelRc::new(VecModel::from(rows)));
            }
            Err(e) => self.fail(&w, e),
        }
    }

    /// Makes `dir` the last instance and restarts into it.
    fn switch(&self, w: &AppWindow, dir: &Path) {
        match instances::register(&self.registry, dir) {
            Ok(()) => {
                self.switching.set(true);
                let _ = slint::quit_event_loop();
            }
            Err(e) => self.fail(w, e),
        }
    }

    /// Folder picker plus the synced-folder warning; `None` on cancel.
    fn pick(&self, w: &AppWindow) -> Option<PathBuf> {
        let s = w.global::<Strings>();
        let dir = FileDialog::new()
            .set_title(s.get_pick_folder().as_str())
            .pick_folder()?;
        dialogs::confirm_not_synced(&s, &dir).then_some(dir)
    }

    /// Opens the database once so the folder becomes a real instance.
    fn create(&self, w: &AppWindow, dir: &Path) -> bool {
        match db::open(&dir.join(db::FILE_NAME)) {
            Ok(_) => true,
            Err(e) => {
                self.fail(w, e);
                false
            }
        }
    }

    fn new_instance(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        w.set_instances_status("".into());
        let Some(dir) = self.pick(&w) else { return };
        if instances::has_database(&dir) {
            w.set_instances_status(
                w.global::<Strings>()
                    .invoke_already_instance(dir.display().to_string().into()),
            );
        } else if self.create(&w, &dir) {
            self.switch(&w, &dir);
        }
    }

    fn open_folder(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        w.set_instances_status("".into());
        let Some(dir) = self.pick(&w) else { return };
        let existing = instances::has_database(&dir);
        if !dialogs::confirm_new(&w.global::<Strings>(), &dir)
            || (!existing && !self.create(&w, &dir))
        {
            return;
        }
        self.switch(&w, &dir);
    }
}

pub fn init(window: &AppWindow, app_dir: &Path, current: &Path, switching: &Rc<Cell<bool>>) {
    let ctx = Rc::new(Ctx {
        window: window.as_weak(),
        registry: app_dir.join("instances.json"),
        current: current.to_path_buf(),
        switching: switching.clone(),
    });
    window.on_instances_opened({
        let ctx = ctx.clone();
        move || ctx.refresh()
    });
    window.on_open_instance({
        let ctx = ctx.clone();
        move |p| {
            if let Some(w) = ctx.window.upgrade() {
                ctx.switch(&w, Path::new(p.as_str()));
            }
        }
    });
    window.on_remove_instance({
        let ctx = ctx.clone();
        move |p| match instances::remove(&ctx.registry, Path::new(p.as_str())) {
            Ok(()) => ctx.refresh(),
            Err(e) => {
                if let Some(w) = ctx.window.upgrade() {
                    ctx.fail(&w, e);
                }
            }
        }
    });
    window.on_new_instance({
        let ctx = ctx.clone();
        move || ctx.new_instance()
    });
    window.on_open_folder(move || ctx.open_folder());
}
