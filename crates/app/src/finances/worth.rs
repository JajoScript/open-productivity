// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The net worth block of the Accounts tab.

use open_productivity_core::finances;
use slint::{ComponentHandle, VecModel};

use super::state::State;
use super::style;
use crate::{AppWindow, FinanceStore, SummaryBar};

/// How many accounts the block draws as bars.
const BARS: usize = 6;

impl State {
    pub fn load_worth(&self, w: &AppWindow) -> Result<(), String> {
        let worth = finances::net_worth(&self.conn).map_err(|e| e.to_string())?;
        let c = self.main()?;
        let fmt = |v| finances::format_amount(v, c, style());
        let store = w.global::<FinanceStore>();
        store.set_worth_total(fmt(worth.total).into());
        store.set_worth_assets(fmt(worth.assets).into());
        store.set_worth_debts(fmt(worth.debts).into());
        let bars = worth
            .slices
            .into_iter()
            .take(BARS)
            .map(|s| SummaryBar {
                name: s.name.into(),
                total: fmt(s.value).into(),
                width: s.width,
            })
            .collect::<Vec<_>>();
        store.set_worth_bars(VecModel::from_slice(&bars));
        Ok(())
    }
}
