// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The Movements tab: the month's rows, the month navigation and the create, edit and delete callbacks.

use std::rc::Rc;

use open_productivity_core::calendar::{self, Period};
use open_productivity_core::chrono::{Local, NaiveDate};
use open_productivity_core::finances::{
    self, AccountId, CategoryId, FinanceError, Movement, Transaction, TransactionId, Transfer,
    TxKind as CoreKind,
};
use slint::{ComponentHandle, SharedString};

use super::state::State;
use super::style;
use crate::dates::{core_date, ui_date};
use crate::tasks::ui_id;
use crate::{AppWindow, Date, FinanceStore, Strings, TransactionRow, TxKind};

pub(super) fn core_kind(k: TxKind) -> CoreKind {
    match k {
        TxKind::Expense => CoreKind::Expense,
        TxKind::Income => CoreKind::Income,
    }
}

pub(super) fn ui_kind(k: CoreKind) -> TxKind {
    match k {
        CoreKind::Expense => TxKind::Expense,
        CoreKind::Income => TxKind::Income,
    }
}

fn tx_row(t: Transaction) -> Result<TransactionRow, String> {
    let style = style();
    Ok(TransactionRow {
        id: ui_id(t.id.0)?,
        kind: ui_kind(t.kind),
        transfer: false,
        to_account_id: 0,
        to_account: Default::default(),
        account_id: ui_id(t.account_id.0)?,
        account: t.account_name.clone().into(),
        has_category: t.category.is_some(),
        category_id: t.category.map_or(Ok(0), |c| ui_id(c.0))?,
        category: t.category_name.clone().unwrap_or_default().into(),
        date: ui_date(t.date),
        description: t.description.clone().into(),
        amount: finances::format_amount(t.signed(), t.currency, style).into(),
        input: finances::format_input(t.amount, t.currency, style).into(),
    })
}

/// The amount of a transfer has no sign: it is neither income nor expense.
fn transfer_row(t: Transfer) -> Result<TransactionRow, String> {
    let style = style();
    Ok(TransactionRow {
        id: ui_id(t.id.0)?,
        kind: TxKind::Expense,
        transfer: true,
        to_account_id: ui_id(t.to.0)?,
        to_account: t.to_name.into(),
        account_id: ui_id(t.from.0)?,
        account: t.from_name.into(),
        has_category: false,
        category_id: 0,
        category: Default::default(),
        date: ui_date(t.date),
        description: t.description.into(),
        amount: finances::format_amount(t.amount, t.currency, style).into(),
        input: finances::format_input(t.amount, t.currency, style).into(),
    })
}

fn row(m: Movement) -> Result<TransactionRow, String> {
    match m {
        Movement::Tx(t) => tx_row(t),
        Movement::Transfer(t) => transfer_row(t),
    }
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
        move |kind, account, has_category, category, amount, date, description| {
            state.with_date(date, |d| {
                finances::create_transaction(
                    &state.conn,
                    core_kind(kind),
                    AccountId(account.into()),
                    &amount,
                    d,
                    &description,
                    has_category.then_some(CategoryId(category.into())),
                )
                .map(|_| ())
            })
        }
    });

    store.on_update_transaction({
        let state = state.clone();
        move |id, kind, account, has_category, category, amount, date, description| {
            state.with_date(date, |d| {
                finances::update_transaction(
                    &state.conn,
                    TransactionId(id.into()),
                    core_kind(kind),
                    AccountId(account.into()),
                    &amount,
                    d,
                    &description,
                    has_category.then_some(CategoryId(category.into())),
                )
            })
        }
    });

    store.on_create_transfer({
        let state = state.clone();
        move |from, to, amount, date, description| {
            state.with_date(date, |d| {
                finances::create_transfer(
                    &state.conn,
                    AccountId(from.into()),
                    AccountId(to.into()),
                    &amount,
                    d,
                    &description,
                )
                .map(|_| ())
            })
        }
    });

    store.on_update_transfer({
        let state = state.clone();
        move |id, from, to, amount, date, description| {
            state.with_date(date, |d| {
                finances::update_transfer(
                    &state.conn,
                    TransactionId(id.into()),
                    AccountId(from.into()),
                    AccountId(to.into()),
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
