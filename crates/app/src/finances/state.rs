// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! What the Finances view shows: the models, their refresh and the error messages.

use std::cell::Cell;
use std::rc::Rc;

use open_productivity_core::chrono::NaiveDate;
use open_productivity_core::db::Connection;
use open_productivity_core::finances::{
    self, Account, AccountKind as CoreKind, CURRENCIES, FinanceError,
};
use slint::{ComponentHandle, SharedString, VecModel};

use super::style;
use crate::tasks::ui_id;
use crate::{
    AccountKind, AccountRow, AppWindow, CategoryRow, CurrencyRow, FinanceStore, FinanceTab,
    Strings, TransactionRow,
};

pub struct State {
    pub conn: Rc<Connection>,
    /// The Accounts tab: active accounts, then archived ones.
    pub rows: Rc<VecModel<AccountRow>>,
    pub archived_account_rows: Rc<VecModel<AccountRow>>,
    pub tx_rows: Rc<VecModel<TransactionRow>>,
    /// The Categories tab: the kind's active categories, then its archived ones.
    pub cat_rows: Rc<VecModel<CategoryRow>>,
    pub archived_rows: Rc<VecModel<CategoryRow>>,
    /// Every category of both kinds, for the movement modal.
    pub choices: Rc<VecModel<CategoryRow>>,
    /// First day of the month the Movements tab shows.
    pub shown: Cell<NaiveDate>,
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
        archived: a.archived,
        in_use: a.in_use,
    })
}

impl State {
    fn reload(&self, w: &AppWindow) -> Result<(), String> {
        let has_currency = finances::main_currency(&self.conn)
            .map_err(|e| e.to_string())?
            .is_some();
        let (mut active, mut archived) = (Vec::new(), Vec::new());
        if has_currency {
            for a in finances::list_accounts(&self.conn).map_err(|e| e.to_string())? {
                let is_archived = a.archived;
                let r = account_row(a)?;
                if is_archived {
                    archived.push(r);
                } else {
                    active.push(r);
                }
            }
        }
        self.rows.set_vec(active);
        self.archived_account_rows.set_vec(archived);
        let store = w.global::<FinanceStore>();
        store.set_has_currency(has_currency);
        // Only the shown tab's data is read.
        if has_currency {
            match store.get_tab() {
                FinanceTab::Movements => {
                    self.load_month(w)?;
                    self.load_choices()?;
                }
                FinanceTab::Categories => self.load_categories(w)?,
                FinanceTab::Accounts => {}
            }
        }
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

    /// Shows a failure in the view, like a load failure.
    pub fn show_error(&self, e: String) {
        if let Some(w) = self.window.upgrade() {
            let msg = w.global::<Strings>().invoke_finance_load_failed(e.into());
            w.global::<FinanceStore>().set_error(msg);
        }
    }

    /// The user-facing text of an error; only database errors and corrupt stored data show raw.
    fn message(w: &AppWindow, e: FinanceError) -> SharedString {
        let s = w.global::<Strings>();
        match e {
            FinanceError::EmptyName => s.get_finance_empty_name(),
            FinanceError::InvalidAmount => s.get_finance_invalid_amount(),
            FinanceError::NoMainCurrency => s.get_finance_no_currency(),
            FinanceError::AccountNotFound => s.get_finance_no_account(),
            FinanceError::TransactionNotFound => s.get_finance_no_transaction(),
            FinanceError::CategoryNotFound => s.get_finance_no_category(),
            FinanceError::DuplicateName => s.get_finance_duplicate_name(),
            FinanceError::InUse => s.get_finance_category_in_use(),
            FinanceError::CategoriesExist => s.get_finance_categories_exist(),
            FinanceError::CategoryKindMismatch => s.get_finance_category_kind(),
            FinanceError::CategoryArchived => s.get_finance_category_archived(),
            FinanceError::SameAccount => s.get_finance_same_account(),
            FinanceError::AccountInUse => s.get_finance_account_in_use(),
            FinanceError::AccountArchived => s.get_finance_account_archived(),
            e @ (FinanceError::Sqlite(_)
            | FinanceError::UnknownCurrency(_)
            | FinanceError::BadKind(_)
            | FinanceError::BadTxKind(_)
            | FinanceError::BadDate(_)) => s.invoke_finance_failed(e.to_string().into()),
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
