// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The net worth card of the Accounts tab: the total and the donut of the positive accounts.

use std::rc::Rc;

use open_productivity_core::finances;
use slint::{ComponentHandle, ModelRc, VecModel};

use super::state::State;
use super::style;
use crate::{AppWindow, DonutSliceRow, FinanceStore};

pub fn connect(state: &Rc<State>, store: &FinanceStore<'_>) {
    store.on_worth_donut_path({
        let state = state.clone();
        move |i, w, h, _| {
            let index = usize::try_from(i).ok();
            finances::donut_path(&state.charts.borrow().worth_shares, w, h, index).into()
        }
    });
}

impl State {
    pub fn load_worth(&self, w: &AppWindow) -> Result<(), String> {
        let worth = finances::net_worth(&self.conn).map_err(|e| e.to_string())?;
        let c = self.main()?;
        let store = w.global::<FinanceStore>();
        store.set_worth_total(finances::format_amount(worth.total, c, style()).into());
        let slices = worth.slices.iter().map(|s| DonutSliceRow {
            name: s.name.as_str().into(),
            percent: match s.percent {
                0 => "<1%".into(),
                p => format!("{p}%").into(),
            },
        });
        store.set_worth_slices(ModelRc::new(VecModel::from(slices.collect::<Vec<_>>())));
        self.charts.borrow_mut().worth_shares = worth.slices.iter().map(|s| s.share).collect();
        store.set_chart_version(store.get_chart_version().wrapping_add(1));
        Ok(())
    }
}
