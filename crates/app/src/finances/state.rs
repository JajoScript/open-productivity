// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! What the Finances view shows: the models, their refresh and the error messages.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use open_productivity_core::chrono::NaiveDate;
use open_productivity_core::db::Connection;
use open_productivity_core::finances::{
    self, Account, AccountKind as CoreKind, Bill, CURRENCIES, Currency, FinanceError, Rates,
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::style;
use crate::dates::ui_date;
use crate::tasks::ui_id;
use crate::{
    AccountKind, AccountRow, AppWindow, BillRow, CategoryRow, CurrencyRow, FinanceStore,
    FinanceTab, RateRow, ScheduledRow, Strings, TransactionRow,
};
use open_productivity_core::chrono::Local;

pub struct State {
    pub conn: Rc<Connection>,
    /// The Accounts tab: active accounts, then archived ones.
    pub rows: Rc<VecModel<AccountRow>>,
    pub archived_account_rows: Rc<VecModel<AccountRow>>,
    pub tx_rows: Rc<VecModel<TransactionRow>>,
    /// The To pay tab: what is due in the shown month.
    pub due_rows: Rc<VecModel<ScheduledRow>>,
    /// The To pay tab: the card bills due in the shown month.
    pub bill_rows: Rc<VecModel<BillRow>>,
    /// The Categories tab: the kind's active categories, then its archived ones.
    pub cat_rows: Rc<VecModel<CategoryRow>>,
    pub archived_rows: Rc<VecModel<CategoryRow>>,
    /// Every category of both kinds, for the movement modal.
    pub choices: Rc<VecModel<CategoryRow>>,
    /// First day of the month the Movements tab shows.
    pub shown: Cell<NaiveDate>,
    /// The exchange rates edited in the open modal (code, text), saved together.
    pub pending_rates: RefCell<Vec<(String, String)>>,
    pub window: slint::Weak<AppWindow>,
}

pub fn core_kind(k: AccountKind) -> CoreKind {
    match k {
        AccountKind::Checking => CoreKind::Checking,
        AccountKind::Demand => CoreKind::Demand,
        AccountKind::Savings => CoreKind::Savings,
        AccountKind::Cash => CoreKind::Cash,
        AccountKind::Credit => CoreKind::Credit,
        AccountKind::CreditLine => CoreKind::CreditLine,
    }
}

fn ui_kind(k: CoreKind) -> AccountKind {
    match k {
        CoreKind::Checking => AccountKind::Checking,
        CoreKind::Demand => AccountKind::Demand,
        CoreKind::Savings => AccountKind::Savings,
        CoreKind::Cash => AccountKind::Cash,
        CoreKind::Credit => AccountKind::Credit,
        CoreKind::CreditLine => AccountKind::CreditLine,
    }
}

/// The currencies to pick from; `rates` tells which still need an exchange rate.
pub fn currency_rows(rates: Option<&Rates>) -> Vec<CurrencyRow> {
    CURRENCIES
        .iter()
        .map(|c| CurrencyRow {
            code: c.code.into(),
            symbol: c.symbol.into(),
            needs_rate: rates.is_some_and(|r| r.needs_rate(c)),
        })
        .collect()
}

fn account_row(a: Account, main: &Currency, bill: Option<&Bill>) -> Result<AccountRow, String> {
    let style = style();
    let (card, usage) = (a.card, a.credit_usage());
    let money = |v: i64| finances::format_in(v, a.currency, main, style);
    Ok(AccountRow {
        limit: a
            .credit_limit
            .map_or_else(String::new, |l| {
                finances::format_input(l, a.currency, style)
            })
            .into(),
        limit_text: a.credit_limit.map_or_else(String::new, money).into(),
        closing_day: card.map_or(0, |t| t.closing_day as i32),
        due_day: card.map_or(0, |t| t.due_day as i32),
        used: usage.map_or_else(String::new, |u| money(u.used)).into(),
        available: usage
            .map_or_else(String::new, |u| money(u.available))
            .into(),
        bill: bill.map_or_else(String::new, |b| money(b.pending)).into(),
        bill_due: bill.map_or_else(Default::default, |b| ui_date(b.due)),
        use_permille: usage.map_or(0, |u| i32::try_from(u.per_mille).unwrap_or(i32::MAX)),
        over_limit: usage.is_some_and(|u| u.over_limit),
        id: ui_id(a.id.0)?,
        name: a.name.into(),
        kind: ui_kind(a.kind),
        bank: a.bank.into(),
        currency: a.currency.code.into(),
        balance: finances::format_in(a.balance, a.currency, main, style).into(),
        initial: finances::format_input(a.initial_balance, a.currency, style).into(),
        archived: a.archived,
        in_use: a.in_use,
    })
}

impl State {
    fn reload(&self, w: &AppWindow) -> Result<(), String> {
        let main = finances::main_currency(&self.conn).map_err(|e| e.to_string())?;
        let has_currency = main.is_some();
        let store = w.global::<FinanceStore>();
        let (mut active, mut archived) = (Vec::new(), Vec::new());
        if let Some(main) = main {
            let rates = finances::load_rates(&self.conn).map_err(|e| e.to_string())?;
            store.set_currencies(ModelRc::new(VecModel::from(currency_rows(Some(&rates)))));
            store.set_main_currency(main.code.into());
            let in_use = finances::list_rates(&self.conn)
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|r| RateRow {
                    code: r.currency.code.into(),
                    rate: r
                        .rate
                        .map_or_else(String::new, |v| finances::format_rate(v, style()))
                        .into(),
                })
                .collect::<Vec<_>>();
            store.set_rates(ModelRc::new(VecModel::from(in_use)));
            // Only the Accounts tab shows the cards' bills.
            let bills = if store.get_tab() == FinanceTab::Accounts {
                finances::list_bills(&self.conn, Local::now().date_naive())
                    .map_err(|e| e.to_string())?
            } else {
                Vec::new()
            };
            for a in finances::list_accounts(&self.conn).map_err(|e| e.to_string())? {
                let is_archived = a.archived;
                let bill = bills.iter().find(|b| b.card == a.id);
                let r = account_row(a, main, bill)?;
                if is_archived {
                    archived.push(r);
                } else {
                    active.push(r);
                }
            }
        }
        store.set_has_cards(
            active
                .iter()
                .any(|r| matches!(r.kind, AccountKind::Credit | AccountKind::CreditLine)),
        );
        self.rows.set_vec(active);
        self.archived_account_rows.set_vec(archived);
        store.set_has_currency(has_currency);
        // Only the shown tab's data is read.
        if has_currency {
            match store.get_tab() {
                FinanceTab::Movements => {
                    self.load_month(w)?;
                    self.load_choices()?;
                }
                FinanceTab::Due => {
                    self.load_due(w)?;
                    self.load_choices()?;
                }
                FinanceTab::Summary => self.load_summary(w)?,
                FinanceTab::Categories => self.load_categories(w)?,
                FinanceTab::Accounts => {}
            }
        }
        Ok(())
    }

    /// The instance's main currency.
    pub fn main(&self) -> Result<&'static Currency, String> {
        finances::main_currency(&self.conn)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no main currency".to_string())
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
            FinanceError::MissingRate(code) => s.invoke_finance_missing_rate(code.into()),
            FinanceError::InvalidRate => s.get_finance_invalid_rate(),
            FinanceError::CurrencyLocked => s.get_finance_currency_locked(),
            FinanceError::InvalidLimit => s.get_finance_invalid_limit(),
            FinanceError::InvalidDay => s.get_finance_invalid_day(),
            FinanceError::KindLocked => s.get_finance_kind_locked(),
            FinanceError::InvalidSchedule => s.get_finance_invalid_schedule(),
            FinanceError::ScheduledNotFound => s.get_finance_no_scheduled(),
            FinanceError::ScheduledDone => s.get_finance_scheduled_done(),
            FinanceError::AccountHasSchedules(n) => {
                s.invoke_schedules_use_account(i32::try_from(n).unwrap_or(i32::MAX))
            }
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
