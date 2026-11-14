// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The Summary tab: the month's figures, expense bars and latest movements.

use open_productivity_core::finances::{self, Currency, Figure};
use slint::{ComponentHandle, VecModel};

use super::state::State;
use super::style;
use super::transactions::row;
use crate::{AppWindow, FinanceStore, Strings, SummaryBar, SummaryFigure};

/// The amount and its change versus the month before, signed.
fn figure(f: Figure, c: &Currency, s: &Strings<'_>) -> SummaryFigure {
    let style = style();
    let change = f.change();
    SummaryFigure {
        amount: finances::format_amount(f.value, c, style).into(),
        change: match change {
            0 => s.get_same_as_last_month(),
            _ => {
                let sign = if change > 0 { '+' } else { '\u{2212}' };
                let abs = finances::format_amount(change.saturating_abs(), c, style);
                s.invoke_vs_last_month(format!("{sign}{abs}").into())
            }
        },
    }
}

impl State {
    /// Loads the shown month's summary.
    pub fn load_summary(&self, w: &AppWindow) -> Result<(), String> {
        let month = self.shown.get();
        let summary = finances::month_summary(&self.conn, month).map_err(|e| e.to_string())?;
        let c = finances::main_currency(&self.conn)
            .map_err(|e| e.to_string())?
            .ok_or("no main currency")?;
        let (store, s) = (w.global::<FinanceStore>(), w.global::<Strings>());
        store.set_summary_income(figure(summary.income, c, &s));
        store.set_summary_expense(figure(summary.expense, c, &s));
        store.set_summary_balance(figure(summary.balance, c, &s));
        store.set_summary_accounts(figure(summary.accounts_total, c, &s));
        let bars = summary
            .by_category
            .into_iter()
            .map(|b| SummaryBar {
                name: b.name.map_or_else(|| s.get_no_category(), Into::into),
                total: finances::format_amount(b.total, c, style()).into(),
                width: b.width,
            })
            .collect::<Vec<_>>();
        store.set_summary_bars(VecModel::from_slice(&bars));
        let recent = summary
            .recent
            .into_iter()
            .map(row)
            .collect::<Result<Vec<_>, _>>()?;
        store.set_summary_recent(VecModel::from_slice(&recent));
        store.set_month(super::super::dates::ui_date(month));
        Ok(())
    }
}
