// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The Movements tab: the month's rows, the month navigation and the create, edit and delete callbacks.

use std::rc::Rc;

use open_productivity_core::calendar::{self, Period};
use open_productivity_core::chrono::{Local, NaiveDate};
use open_productivity_core::finances::{
    self, AccountId, FinanceError, Transaction, TransactionId, TxKind as CoreKind,
};
use slint::{ComponentHandle, SharedString};

use super::state::State;
use super::style;
use crate::dates::{core_date, ui_date};
use crate::tasks::ui_id;
use crate::{AppWindow, Date, FinanceStore, Strings, TransactionRow, TxKind};

fn core_kind(k: TxKind) -> CoreKind {
    match k {
        TxKind::Expense => CoreKind::Expense,
        TxKind::Income => CoreKind::Income,
    }
}

fn ui_kind(k: CoreKind) -> TxKind {
    match k {
        CoreKind::Expense => TxKind::Expense,
        CoreKind::Income => TxKind::Income,
    }
}

fn row(t: Transaction) -> Result<TransactionRow, String> {
    let style = style();
    Ok(TransactionRow {
        id: ui_id(t.id.0)?,
        kind: ui_kind(t.kind),
        account_id: ui_id(t.account_id.0)?,
        account: t.account_name.clone().into(),
        date: ui_date(t.date),
        description: t.description.clone().into(),
        amount: finances::format_amount(t.signed(), t.currency, style).into(),
        input: finances::format_input(t.amount, t.currency, style).into(),
    })
}

impl State {
    /// Loads the shown month into the list and the title.
    pub fn load_month(&self, w: &AppWindow) -> Result<(), String> {
        let month = self.shown.get();
        let rows = finances::list_month(&self.conn, month)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(row)
            .collect::<Result<Vec<_>, _>>()?;
        self.tx_rows.set_vec(rows);
        w.global::<FinanceStore>().set_month(ui_date(month));
        Ok(())
    }

    /// Runs a save with the picked date; a date the picker cannot produce is an error, never "today".
    fn with_date(
        &self,
        date: Date,
        op: impl FnOnce(NaiveDate) -> Result<(), FinanceError>,
    ) -> SharedString {
        match core_date(date) {
            Ok(d) => self.finish(op(d)),
            Err(e) => match self.window.upgrade() {
                Some(w) => w.global::<Strings>().invoke_finance_failed(e.into()),
                None => Default::default(),
            },
        }
    }
}

pub fn connect(state: &Rc<State>, store: &FinanceStore<'_>) {
    store.on_create_transaction({
        let state = state.clone();
        move |kind, account, amount, date, description| {
            state.with_date(date, |d| {
                finances::create_transaction(
                    &state.conn,
                    core_kind(kind),
                    AccountId(account.into()),
                    &amount,
                    d,
                    &description,
                )
                .map(|_| ())
            })
        }
    });

    store.on_update_transaction({
        let state = state.clone();
        move |id, kind, account, amount, date, description| {
            state.with_date(date, |d| {
                finances::update_transaction(
                    &state.conn,
                    TransactionId(id.into()),
                    core_kind(kind),
                    AccountId(account.into()),
                    &amount,
                    d,
                    &description,
                )
            })
        }
    });

    store.on_delete_transaction({
        let state = state.clone();
        move |id| {
            state.finish(finances::delete_transaction(
                &state.conn,
                TransactionId(id.into()),
            ))
        }
    });

    // 0 goes back to the current month; otherwise moves that many months.
    store.on_shift({
        let state = state.clone();
        move |months| {
            let today = Local::now().date_naive();
            match calendar::navigate(state.shown.get(), Period::Month, months, today) {
                Some(first) => {
                    state.shown.set(calendar::month_start(first));
                    state.show_load();
                }
                None => state.show_error("date out of range".into()),
            }
        }
    });
}
