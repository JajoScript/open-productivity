// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! Net worth: every active account in the main currency, assets against debts.

use rusqlite::Connection;

use super::FinanceError;
use super::accounts::{AccountId, list_accounts};
use super::rates::load_rates;

/// An account with a positive value.
#[derive(Debug, PartialEq)]
pub struct WorthSlice {
    pub account: AccountId,
    pub name: String,
    pub value: i64,
    /// Value relative to the biggest slice, in (0, 1].
    pub width: f32,
}

#[derive(Debug, PartialEq)]
pub struct NetWorth {
    /// Assets minus debts.
    pub total: i64,
    pub assets: i64,
    /// The negative balances (cards and lines included), as a positive number.
    pub debts: i64,
    /// Accounts with a positive value, biggest first then by name.
    pub slices: Vec<WorthSlice>,
}

/// The net worth of the non-archived accounts, in the main currency.
pub fn net_worth(conn: &Connection) -> Result<NetWorth, FinanceError> {
    let rates = load_rates(conn)?;
    let (mut assets, mut debts) = (0i64, 0i64);
    let mut slices = Vec::new();
    for a in list_accounts(conn)?.into_iter().filter(|a| !a.archived) {
        let value = rates.to_main(a.balance, a.currency)?;
        if value > 0 {
            assets = assets.saturating_add(value);
            slices.push(WorthSlice {
                account: a.id,
                name: a.name,
                value,
                width: 1.0,
            });
        } else {
            debts = debts.saturating_add(value.saturating_neg());
        }
    }
    slices.sort_by(|a, b| b.value.cmp(&a.value).then_with(|| a.name.cmp(&b.name)));
    let max = slices.first().map_or(1, |s| s.value);
    for s in &mut slices {
        s.width = s.value as f32 / max as f32;
    }
    Ok(NetWorth {
        total: assets.saturating_sub(debts),
        assets,
        debts,
        slices,
    })
}
