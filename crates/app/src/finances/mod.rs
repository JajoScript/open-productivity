// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Glue between the Finances view and `core::finances`.

mod categories;
mod state;
mod transactions;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use open_productivity_core::calendar;
use open_productivity_core::chrono::Local;
use open_productivity_core::db::Connection;
use open_productivity_core::finances::{self, AccountId, AmountStyle};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::{AppWindow, FinanceStore};
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

pub fn init(window: &AppWindow, conn: Rc<Connection>) {
    let state = Rc::new(State {
        conn,
        rows: Rc::default(),
        archived_account_rows: Rc::default(),
        tx_rows: Rc::default(),
        cat_rows: Rc::default(),
        archived_rows: Rc::default(),
        choices: Rc::default(),
        shown: Cell::new(calendar::month_start(Local::now().date_naive())),
        window: window.as_weak(),
    });
    let store = window.global::<FinanceStore>();
    store.set_accounts(ModelRc::from(state.rows.clone()));
    store.set_archived_accounts(ModelRc::from(state.archived_account_rows.clone()));
    store.set_transactions(ModelRc::from(state.tx_rows.clone()));
    store.set_categories(ModelRc::from(state.cat_rows.clone()));
    store.set_archived_categories(ModelRc::from(state.archived_rows.clone()));
    store.set_category_choices(ModelRc::from(state.choices.clone()));
    store.set_currencies(ModelRc::new(VecModel::from(state::currency_rows())));
    state.show_load();

    store.on_refresh({
        let state = state.clone();
        move || state.show_load()
    });

    transactions::connect(&state, &store);
    categories::connect(&state, &store);

    store.on_choose_currency({
        let state = state.clone();
        move |code| state.finish(finances::set_main_currency(&state.conn, &code))
    });

    store.on_create_account({
        let state = state.clone();
        move |name, kind, bank, balance| {
            state.finish(
                finances::create_account(&state.conn, &name, core_kind(kind), &bank, &balance)
                    .map(|_| ()),
            )
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
        move |id, name, kind, bank, balance| {
            state.finish(finances::update_account(
                &state.conn,
                AccountId(id.into()),
                &name,
                core_kind(kind),
                &bank,
                &balance,
            ))
        }
    });
}
