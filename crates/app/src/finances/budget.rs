// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The Budget tab: the month's buckets with what each has spent.

use std::rc::Rc;

use open_productivity_core::finances::{self, Bucket, BucketId, BudgetBase, CategoryId};
use slint::ComponentHandle;

use super::state::State;
use super::style;
use crate::dates::ui_date;
use crate::tasks::ui_id;
use crate::{AppWindow, BucketRow, FinanceStore, Strings};

/// A bucket that is not in the month's budget: only its name and state show.
fn archived_row(b: &Bucket) -> Result<BucketRow, String> {
    Ok(BucketRow {
        id: ui_id(b.id.0)?,
        name: b.name.as_str().into(),
        archived: true,
        in_use: b.in_use,
        ..Default::default()
    })
}

impl State {
    /// Loads the shown month's budget.
    pub fn load_budget(&self, w: &AppWindow) -> Result<(), String> {
        let month = self.shown.get();
        let summary = finances::budget_summary(&self.conn, month).map_err(|e| e.to_string())?;
        let all = finances::list_buckets(&self.conn).map_err(|e| e.to_string())?;
        let setting = finances::budget_base(&self.conn).map_err(|e| e.to_string())?;
        let (c, style) = (self.main()?, style());
        let money = |v: i64| finances::format_amount(v, c, style);
        let active = summary
            .buckets
            .iter()
            .map(|b| {
                Ok(BucketRow {
                    id: ui_id(b.id.0)?,
                    name: b.name.as_str().into(),
                    percent: b.percent as i32,
                    assigned: money(b.assigned).into(),
                    spent: money(b.spent).into(),
                    // Negative is shown as the amount over, so the sign is not read twice.
                    remaining: money(b.remaining.saturating_abs()).into(),
                    overspent: b.overspent,
                    spent_permille: i32::try_from(b.spent_per_mille).unwrap_or(i32::MAX),
                    archived: false,
                    in_use: all.iter().any(|x| x.id == b.id && x.in_use),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let archived = all
            .iter()
            .filter(|b| b.archived)
            .map(archived_row)
            .collect::<Result<Vec<_>, _>>()?;
        let store = w.global::<FinanceStore>();
        store.set_has_buckets(!all.is_empty());
        store.set_budget_previous(setting == BudgetBase::Previous);
        store.set_budget_base(money(summary.base).into());
        store.set_budget_base_month(ui_date(summary.base_month));
        store.set_budget_unassigned(money(summary.unassigned).into());
        store.set_month(ui_date(month));
        self.bucket_rows.set_vec(active);
        self.archived_bucket_rows.set_vec(archived);
        Ok(())
    }
}

pub fn connect(state: &Rc<State>, store: &FinanceStore<'_>) {
    store.on_create_bucket({
        let state = state.clone();
        move |name| state.finish(finances::create_bucket(&state.conn, &name).map(|_| ()))
    });

    store.on_rename_bucket({
        let state = state.clone();
        move |id, name| {
            state.finish(finances::rename_bucket(
                &state.conn,
                BucketId(id.into()),
                &name,
            ))
        }
    });

    // The typed percent; anything that is not a whole number becomes -1, which `core` rejects.
    store.on_set_share({
        let state = state.clone();
        move |id, percent| {
            let percent = percent.trim().trim_end_matches('%').parse().unwrap_or(-1);
            state.finish(finances::set_share(
                &state.conn,
                state.shown.get(),
                BucketId(id.into()),
                percent,
            ))
        }
    });

    store.on_archive_bucket({
        let state = state.clone();
        move |id, archived| {
            state.finish(finances::set_bucket_archived(
                &state.conn,
                BucketId(id.into()),
                archived,
            ))
        }
    });

    store.on_delete_bucket({
        let state = state.clone();
        move |id| state.finish(finances::delete_bucket(&state.conn, BucketId(id.into())))
    });

    // `bucket` is the bucket the category is put in, or taken out of.
    store.on_assign_category({
        let state = state.clone();
        move |category, bucket, assigned| {
            let bucket = assigned.then(|| BucketId(bucket.into()));
            state.finish(finances::set_category_bucket(
                &state.conn,
                CategoryId(category.into()),
                bucket,
            ))
        }
    });

    store.on_set_budget_base({
        let state = state.clone();
        move |previous| {
            let base = if previous {
                BudgetBase::Previous
            } else {
                BudgetBase::Current
            };
            state.finish(finances::set_budget_base(&state.conn, base))
        }
    });

    store.on_create_suggested_budget({
        let state = state.clone();
        move || {
            let Some(w) = state.window.upgrade() else {
                return Default::default();
            };
            let s = w.global::<Strings>();
            let names = [
                s.get_bucket_needs(),
                s.get_bucket_wants(),
                s.get_bucket_savings(),
            ];
            state.finish(finances::create_suggested_budget(
                &state.conn,
                state.shown.get(),
                [&names[0], &names[1], &names[2]],
            ))
        }
    });
}
