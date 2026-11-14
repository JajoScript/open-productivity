// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The reserves of the Budget tab and their modals' callbacks.

use std::rc::Rc;

use open_productivity_core::finances::{self, AccountId, Currency, Reserve, ReserveId};
use slint::{ComponentHandle, ModelRc, VecModel};

use super::state::State;
use super::style;
use crate::dates::ui_date;
use crate::tasks::ui_id;
use crate::{AppWindow, FinanceStore, ReserveDetail, ReserveMoveRow, ReserveRow};

fn reserve_row(r: Reserve, main: &Currency) -> Result<ReserveRow, String> {
    Ok(ReserveRow {
        id: ui_id(r.id.0)?,
        name: r.name.into(),
        account_name: r.account_name.into(),
        currency: r.currency.code.into(),
        balance: finances::format_in(r.balance, r.currency, main, style()).into(),
        archived: r.archived,
        account_archived: r.account_archived,
    })
}

impl State {
    /// Loads the active reserves, then the archived ones.
    pub fn load_reserves(&self, w: &AppWindow) -> Result<(), String> {
        let main = self.main()?;
        let rows = finances::list_reserves(&self.conn)
            .and_then(|a| Ok((a, finances::list_archived_reserves(&self.conn)?)))
            .map_err(|e| e.to_string())?;
        let rows = rows
            .0
            .into_iter()
            .chain(rows.1)
            .map(|r| reserve_row(r, main))
            .collect::<Result<Vec<_>, _>>()?;
        self.reserve_rows.set_vec(rows);
        let total = finances::reserved_total(&self.conn).map_err(|e| e.to_string())?;
        w.global::<FinanceStore>()
            .set_reserved_total(finances::format_amount(total, main, style()).into());
        Ok(())
    }

    fn detail(&self, id: ReserveId) -> Result<ReserveDetail, String> {
        let main = self.main()?;
        let r = finances::get_reserve(&self.conn, id).map_err(|e| e.to_string())?;
        let moves = finances::list_moves(&self.conn, id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|m| ReserveMoveRow {
                date: ui_date(m.date),
                description: m.description.into(),
                amount: finances::format_in(m.amount, r.currency, main, style()).into(),
                withdraw: m.withdraw,
            })
            .collect::<Vec<_>>();
        Ok(ReserveDetail {
            balance: finances::format_in(r.balance, r.currency, main, style()).into(),
            empty: r.balance == 0,
            in_use: r.in_use,
            moves: ModelRc::new(VecModel::from(moves)),
        })
    }
}

pub fn connect(state: &Rc<State>, store: &FinanceStore<'_>) {
    store.on_create_reserve({
        let state = state.clone();
        move |name, account| {
            state.finish(
                finances::create_reserve(&state.conn, &name, AccountId(account.into())).map(|_| ()),
            )
        }
    });

    store.on_rename_reserve({
        let state = state.clone();
        move |id, name| {
            state.finish(finances::rename_reserve(
                &state.conn,
                ReserveId(id.into()),
                &name,
            ))
        }
    });

    store.on_archive_reserve({
        let state = state.clone();
        move |id, archived| {
            state.finish(finances::set_reserve_archived(
                &state.conn,
                ReserveId(id.into()),
                archived,
            ))
        }
    });

    store.on_delete_reserve({
        let state = state.clone();
        move |id| state.finish(finances::delete_reserve(&state.conn, ReserveId(id.into())))
    });

    store.on_reserve_money({
        let state = state.clone();
        move |id, amount, date, description| {
            state.with_date(date, |d| {
                finances::reserve_money(&state.conn, ReserveId(id.into()), &amount, d, &description)
            })
        }
    });

    store.on_withdraw_reserve({
        let state = state.clone();
        move |id, amount, date, description| {
            state.with_date(date, |d| {
                finances::withdraw_from_reserve(
                    &state.conn,
                    ReserveId(id.into()),
                    &amount,
                    d,
                    &description,
                )
            })
        }
    });

    store.on_reserve_detail({
        let state = state.clone();
        move |id| match state.detail(ReserveId(id.into())) {
            Ok(d) => d,
            Err(e) => {
                state.show_error(e);
                ReserveDetail {
                    balance: Default::default(),
                    empty: false,
                    in_use: true,
                    moves: Default::default(),
                }
            }
        }
    });
}
