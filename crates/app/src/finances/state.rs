// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! What the Finances view shows: the models, their refresh and the error messages.

use std::rc::Rc;

use open_productivity_core::db::Connection;
use open_productivity_core::finances::{
    self, Account, AccountKind as CoreKind, CURRENCIES, FinanceError,
};
use slint::{ComponentHandle, SharedString, VecModel};

use super::style;
use crate::tasks::ui_id;
use crate::{AccountKind, AccountRow, AppWindow, CurrencyRow, FinanceStore, Strings};

pub struct State {
    pub conn: Rc<Connection>,
    pub rows: Rc<VecModel<AccountRow>>,
    pub window: slint::Weak<AppWindow>,
}

pub fn core_kind(k: AccountKind) -> CoreKind {
    match k {
        AccountKind::Checking => CoreKind::Checking,
        AccountKind::Demand => CoreKind::Demand,
        AccountKind::Savings => CoreKind::Savings,
        AccountKind::Cash => CoreKind::Cash,
    }
}

fn ui_kind(k: CoreKind) -> AccountKind {
    match k {
        CoreKind::Checking => AccountKind::Checking,
        CoreKind::Demand => AccountKind::Demand,
        CoreKind::Savings => AccountKind::Savings,
        CoreKind::Cash => AccountKind::Cash,
    }
}

pub fn currency_rows() -> Vec<CurrencyRow> {
    CURRENCIES
        .iter()
        .map(|c| CurrencyRow {
            code: c.code.into(),
            symbol: c.symbol.into(),
        })
        .collect()
}

fn account_row(a: Account) -> Result<AccountRow, String> {
    let style = style();
    Ok(AccountRow {
        id: ui_id(a.id.0)?,
        name: a.name.into(),
        kind: ui_kind(a.kind),
        bank: a.bank.into(),
        balance: finances::format_amount(a.balance, a.currency, style).into(),
        initial: finances::format_input(a.initial_balance, a.currency, style).into(),
    })
}

impl State {
    fn reload(&self, w: &AppWindow) -> Result<(), String> {
        let has_currency = finances::main_currency(&self.conn)
            .map_err(|e| e.to_string())?
            .is_some();
        let rows = if has_currency {
            finances::list_accounts(&self.conn)
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(account_row)
                .collect::<Result<Vec<_>, _>>()?
        } else {
            Vec::new()
        };
        self.rows.set_vec(rows);
        w.global::<FinanceStore>().set_has_currency(has_currency);
        Ok(())
    }

    /// Shows a load failure in the view, or clears the last one.
    pub fn show_load(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let msg = match self.reload(&w) {
            Err(e) => w.global::<Strings>().invoke_finance_load_failed(e.into()),
            Ok(()) => Default::default(),
        };
        w.global::<FinanceStore>().set_error(msg);
    }

    /// The user-facing text of an error; only database errors and corrupt stored data show raw.
    fn message(w: &AppWindow, e: FinanceError) -> SharedString {
        let s = w.global::<Strings>();
        match e {
            FinanceError::EmptyName => s.get_finance_empty_name(),
            FinanceError::InvalidAmount => s.get_finance_invalid_amount(),
            FinanceError::NoMainCurrency => s.get_finance_no_currency(),
            FinanceError::AccountNotFound => s.get_finance_no_account(),
            e @ (FinanceError::Sqlite(_)
            | FinanceError::UnknownCurrency(_)
            | FinanceError::BadKind(_)) => s.invoke_finance_failed(e.to_string().into()),
        }
    }

    /// The modal's message for the outcome; empty means it worked, and the view is refreshed.
    pub fn finish(&self, res: Result<(), FinanceError>) -> SharedString {
        let Some(w) = self.window.upgrade() else {
            return Default::default();
        };
        match res {
            Err(e) => Self::message(&w, e),
            Ok(()) => {
                self.show_load();
                Default::default()
            }
        }
    }
}
