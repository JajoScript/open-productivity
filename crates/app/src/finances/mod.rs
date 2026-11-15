// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Finances view and `core::finances`.

mod bills;
mod budget;
mod categories;
mod reserves;
mod scheduled;
mod state;
mod summary;
mod transactions;
mod worth;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use open_productivity_core::calendar;
use open_productivity_core::chrono::Local;
use open_productivity_core::db::Connection;
use open_productivity_core::finances::{self, AccountId, AmountStyle};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{AppWindow, FinanceStore, RateRow};
use state::{State, core_kind};

static SPANISH: AtomicBool = AtomicBool::new(false);

/// Records the language the interface now shows, so amounts follow its separators.
/// `locale` is a code like "es" or "es-CL"; anything else reads as English.
pub fn set_style_for(locale: &str) {
    SPANISH.store(locale.starts_with("es"), Ordering::Relaxed);
}

pub(crate) fn style() -> AmountStyle {
    if SPANISH.load(Ordering::Relaxed) {
        AmountStyle::Spanish
    } else {
        AmountStyle::English
    }
}

/// A day field's text; anything that is not a number becomes 0, which `core` rejects.
fn day(text: &str) -> u32 {
    text.trim().parse().unwrap_or(0)
}

pub fn init(window: &AppWindow, conn: Rc<Connection>) {
    let state = Rc::new(State {
        conn,
        rows: Rc::default(),
        archived_account_rows: Rc::default(),
        tx_rows: Rc::default(),
        due_rows: Rc::default(),
        bill_rows: Rc::default(),
        reserve_rows: Rc::default(),
        bucket_rows: Rc::default(),
        archived_bucket_rows: Rc::default(),
        choices: Rc::default(),
        summary_account_rows: Rc::default(),
        charts: RefCell::default(),
        shown: Cell::new(calendar::month_start(Local::now().date_naive())),
        pending_rates: RefCell::default(),
        window: window.as_weak(),
    });
    let store = window.global::<FinanceStore>();
    store.set_accounts(ModelRc::from(state.rows.clone()));
    store.set_archived_accounts(ModelRc::from(state.archived_account_rows.clone()));
    store.set_transactions(ModelRc::from(state.tx_rows.clone()));
    store.set_summary_accounts(ModelRc::from(state.summary_account_rows.clone()));
    store.set_due(ModelRc::from(state.due_rows.clone()));
    store.set_bills(ModelRc::from(state.bill_rows.clone()));
    store.set_reserves(ModelRc::from(state.reserve_rows.clone()));
    store.set_buckets(ModelRc::from(state.bucket_rows.clone()));
    store.set_archived_buckets(ModelRc::from(state.archived_bucket_rows.clone()));
    store.set_category_choices(ModelRc::from(state.choices.clone()));
    store.set_currencies(ModelRc::new(VecModel::from(state::currency_rows(None))));
    state.show_load();

    store.on_refresh({
        let state = state.clone();
        move || state.show_load()
    });

    store.on_prepare_forms({
        let state = state.clone();
        move || state.prepare_forms()
    });

    transactions::connect(&state, &store);
    summary::connect(&state, &store);
    worth::connect(&state, &store);
    scheduled::connect(&state, &store);
    categories::connect(&state, &store);
    budget::connect(&state, &store);
    reserves::connect(&state, &store);

    store.on_choose_currency({
        let state = state.clone();
        move |code| state.finish(finances::set_main_currency(&state.conn, &code))
    });

    store.on_create_account({
        let state = state.clone();
        move |name, kind, bank, currency, balance, rate| {
            state.finish(
                finances::create_account(
                    &state.conn,
                    &name,
                    core_kind(kind),
                    &bank,
                    &currency,
                    &balance,
                    Some(&rate),
                )
                .map(|_| ()),
            )
        }
    });

    store.on_create_credit_line({
        let state = state.clone();
        move |name, bank, currency, balance, limit, rate| {
            state.finish(
                finances::create_credit_line(
                    &state.conn,
                    &name,
                    &bank,
                    &currency,
                    &balance,
                    &limit,
                    Some(&rate),
                )
                .map(|_| ()),
            )
        }
    });

    store.on_update_credit_line({
        let state = state.clone();
        move |id, name, bank, currency, balance, limit, rate| {
            state.finish(finances::update_credit_line(
                &state.conn,
                AccountId(id.into()),
                &name,
                &bank,
                &currency,
                &balance,
                &limit,
                Some(&rate),
            ))
        }
    });

    store.on_create_card({
        let state = state.clone();
        move |name, bank, currency, limit, closing, due, rate| {
            state.finish(
                finances::create_card(
                    &state.conn,
                    &name,
                    &bank,
                    &currency,
                    &limit,
                    day(&closing),
                    day(&due),
                    Some(&rate),
                )
                .map(|_| ()),
            )
        }
    });

    store.on_update_card({
        let state = state.clone();
        move |id, name, bank, currency, limit, closing, due, rate| {
            state.finish(finances::update_card(
                &state.conn,
                AccountId(id.into()),
                &name,
                &bank,
                &currency,
                &limit,
                day(&closing),
                day(&due),
                Some(&rate),
            ))
        }
    });

    store.on_archive_account({
        let state = state.clone();
        move |id, archived| {
            state.finish(finances::set_account_archived(
                &state.conn,
                AccountId(id.into()),
                archived,
            ))
        }
    });

    store.on_delete_account({
        let state = state.clone();
        move |id| state.finish(finances::delete_account(&state.conn, AccountId(id.into())))
    });

    store.on_update_account({
        let state = state.clone();
        move |id, name, kind, bank, currency, balance, rate| {
            state.finish(finances::update_account(
                &state.conn,
                AccountId(id.into()),
                &name,
                core_kind(kind),
                &bank,
                &currency,
                &balance,
                Some(&rate),
            ))
        }
    });

    // The rates modal reports each edited field; they are saved together.
    store.on_edit_rate({
        let state = state.clone();
        move |code, text| {
            let mut pending = state.pending_rates.borrow_mut();
            pending.retain(|(c, _)| *c != code.as_str());
            pending.push((code.to_string(), text.to_string()));
        }
    });

    store.on_discard_rates({
        let state = state.clone();
        move || state.pending_rates.borrow_mut().clear()
    });

    // Picking a main currency: its rates come suggested, and count as edited until retyped.
    store.on_main_change_rates({
        let state = state.clone();
        move |code| {
            let suggested = finances::main_change_rates(&state.conn, &code).unwrap_or_default();
            let mut pending = state.pending_rates.borrow_mut();
            pending.clear();
            let rows = suggested.into_iter().map(|r| {
                let text = r
                    .rate
                    .map_or_else(String::new, |v| finances::format_rate(v, style()));
                pending.push((r.currency.code.to_string(), text.clone()));
                RateRow {
                    code: r.currency.code.into(),
                    rate: text.into(),
                }
            });
            ModelRc::new(VecModel::from(rows.collect::<Vec<_>>()))
        }
    });

    store.on_change_main_currency({
        let state = state.clone();
        move |code| {
            let pending = state.pending_rates.borrow().clone();
            let list: Vec<(&str, &str)> = pending
                .iter()
                .map(|(c, t)| (c.as_str(), t.as_str()))
                .collect();
            let msg = state.finish(finances::change_main_currency(&state.conn, &code, &list));
            if msg.is_empty() {
                state.pending_rates.borrow_mut().clear();
            }
            msg
        }
    });

    store.on_save_rates({
        let state = state.clone();
        move || {
            let pending = state.pending_rates.borrow().clone();
            let list: Vec<(&str, &str)> = pending
                .iter()
                .map(|(c, t)| (c.as_str(), t.as_str()))
                .collect();
            let msg = state.finish(finances::set_rates(&state.conn, &list));
            if msg.is_empty() {
                state.pending_rates.borrow_mut().clear();
            }
            msg
        }
    });
}
