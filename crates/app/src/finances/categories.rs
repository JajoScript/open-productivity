// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The Categories tab and the category choices of the movement modal.

use std::rc::Rc;

use open_productivity_core::finances::{self, Category, CategoryId, Suggested, TxKind as CoreKind};
use slint::{ComponentHandle, ModelRc, VecModel};

use super::state::State;
use super::style;
use super::transactions::{core_kind, ui_kind};
use crate::tasks::ui_id;
use crate::{AppWindow, BucketRow, CategoryRow, FinanceStore, Strings, SuggestedCategory};

fn ui_suggested(s: Suggested) -> SuggestedCategory {
    match s {
        Suggested::Food => SuggestedCategory::Food,
        Suggested::Transport => SuggestedCategory::Transport,
        Suggested::Housing => SuggestedCategory::Housing,
        Suggested::Bills => SuggestedCategory::Bills,
        Suggested::Health => SuggestedCategory::Health,
        Suggested::Leisure => SuggestedCategory::Leisure,
        Suggested::Shopping => SuggestedCategory::Shopping,
        Suggested::Salary => SuggestedCategory::Salary,
        Suggested::OtherIncome => SuggestedCategory::OtherIncome,
    }
}

fn row(c: Category, total: String, bucket_name: String) -> Result<CategoryRow, String> {
    Ok(CategoryRow {
        id: ui_id(c.id.0)?,
        name: c.name.into(),
        kind: ui_kind(c.kind),
        archived: c.archived,
        in_use: c.in_use,
        bucket_id: c.bucket.map_or(Ok(0), |b| ui_id(b.0))?,
        bucket_name: bucket_name.into(),
        total: total.into(),
    })
}

impl State {
    /// Every category of both kinds, for the movement modal.
    pub fn load_choices(&self) -> Result<(), String> {
        let mut rows = Vec::new();
        for kind in [CoreKind::Expense, CoreKind::Income] {
            for c in finances::list_categories(&self.conn, kind).map_err(|e| e.to_string())? {
                rows.push(row(c, String::new(), String::new())?);
            }
        }
        self.choices.set_vec(rows);
        Ok(())
    }

    /// Loads both kinds' categories with the shown month's totals, and the buckets to pick from.
    pub fn load_categories(&self, w: &AppWindow) -> Result<(), String> {
        let store = w.global::<FinanceStore>();
        let has = finances::has_categories(&self.conn).map_err(|e| e.to_string())?;
        store.set_has_categories(has);
        let currency = self.main()?;
        let style = style();
        let buckets = finances::list_buckets(&self.conn).map_err(|e| e.to_string())?;
        let mut choices = Vec::new();
        for b in buckets.iter().filter(|b| !b.archived) {
            choices.push(BucketRow {
                id: ui_id(b.id.0)?,
                name: b.name.as_str().into(),
                ..Default::default()
            });
        }
        store.set_bucket_choices(ModelRc::new(VecModel::from(choices)));
        let models = |active: Vec<CategoryRow>| ModelRc::new(VecModel::from(active));
        for kind in [CoreKind::Expense, CoreKind::Income] {
            let totals = finances::category_totals(&self.conn, self.shown.get(), kind)
                .map_err(|e| e.to_string())?;
            let total_of = |id: Option<CategoryId>| {
                totals
                    .iter()
                    .find(|t| t.category == id)
                    .map(|t| finances::format_amount(t.total, currency, style))
                    .unwrap_or_default()
            };
            let (mut active, mut archived) = (Vec::new(), Vec::new());
            for c in finances::list_categories(&self.conn, kind).map_err(|e| e.to_string())? {
                let is_archived = c.archived;
                let total = total_of(Some(c.id));
                let bucket = c
                    .bucket
                    .and_then(|id| buckets.iter().find(|b| b.id == id))
                    .map(|b| b.name.clone())
                    .unwrap_or_default();
                let r = row(c, total, bucket)?;
                if is_archived {
                    archived.push(r);
                } else {
                    active.push(r);
                }
            }
            let uncategorized = total_of(None).into();
            if kind == CoreKind::Expense {
                store.set_expense_categories(models(active));
                store.set_archived_expense_categories(models(archived));
                store.set_uncategorized_expense(uncategorized);
            } else {
                store.set_income_categories(models(active));
                store.set_archived_income_categories(models(archived));
                store.set_uncategorized_income(uncategorized);
            }
        }
        Ok(())
    }
}

pub fn connect(state: &Rc<State>, store: &FinanceStore<'_>) {
    store.on_create_category({
        let state = state.clone();
        move |kind, name| {
            state.finish(finances::create_category(&state.conn, core_kind(kind), &name).map(|_| ()))
        }
    });

    store.on_rename_category({
        let state = state.clone();
        move |id, name| {
            state.finish(finances::rename_category(
                &state.conn,
                CategoryId(id.into()),
                &name,
            ))
        }
    });

    store.on_archive_category({
        let state = state.clone();
        move |id, archived| {
            state.finish(finances::set_category_archived(
                &state.conn,
                CategoryId(id.into()),
                archived,
            ))
        }
    });

    store.on_delete_category({
        let state = state.clone();
        move |id| {
            state.finish(finances::delete_category(
                &state.conn,
                CategoryId(id.into()),
            ))
        }
    });

    store.on_create_suggested({
        let state = state.clone();
        move || {
            let Some(w) = state.window.upgrade() else {
                return Default::default();
            };
            let strings = w.global::<Strings>();
            let name = |s| {
                strings
                    .invoke_suggested_category(ui_suggested(s))
                    .to_string()
            };
            state.finish(finances::create_suggested(&state.conn, name))
        }
    });
}
