// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The Summary tab: the four cards, the balance chart, the category donut, the latest movements,
//! the upcoming payments. `core` computes every figure and path; this converts and formats.

use std::rc::Rc;

use open_productivity_core::chrono::{Datelike, Local};
use open_productivity_core::finances::{
    self, BalanceChart, BalanceLayer as CoreLayer, Currency, SummaryCard,
    UpcomingFilter as CoreUpcoming,
};
use slint::{ComponentHandle, ModelRc, VecModel};

use super::state::State;
use super::style;
use super::transactions::{core_filter, core_kind, row};
use crate::dates::ui_date;
use crate::{
    AppWindow, AxisLabelRow, BalanceLayer, DonutSliceRow, FinanceStore, Strings, SummaryCardKind,
    SummaryCardRow, Trend, TxKind, UpcomingBarRow, UpcomingFilter, UpcomingRow,
};

/// How many movements and payments the Summary lists.
const LISTED: usize = 5;

/// What the Summary's chart paths are drawn from: refreshed on every load.
#[derive(Default)]
pub struct Charts {
    /// Running series of the cards: total, balance, income, expenses.
    cards: [Vec<i64>; 4],
    balance: BalanceChart,
    shares: Vec<f32>,
}

fn card_row(
    card: &SummaryCard,
    c: &Currency,
    s: &Strings<'_>,
    accumulated: bool,
) -> SummaryCardRow {
    let (note, trend) = match card.percent {
        _ if accumulated => (s.get_accumulated(), Trend::None),
        None => ("\u{2014}".into(), Trend::None),
        Some(0) => (s.get_same_as_last_month(), Trend::None),
        Some(p) => (
            s.invoke_vs_last_month(format!("{}%", p.unsigned_abs()).into()),
            if p > 0 { Trend::Up } else { Trend::Down },
        ),
    };
    SummaryCardRow {
        amount: finances::format_amount(card.value, c, style()).into(),
        note,
        trend,
    }
}

fn core_upcoming(f: UpcomingFilter) -> CoreUpcoming {
    match f {
        UpcomingFilter::All => CoreUpcoming::All,
        UpcomingFilter::Bills => CoreUpcoming::Bills,
        UpcomingFilter::Installments => CoreUpcoming::Installments,
        UpcomingFilter::Recurring => CoreUpcoming::Recurring,
    }
}

fn core_layer(l: BalanceLayer) -> CoreLayer {
    match l {
        BalanceLayer::Current => CoreLayer::Current,
        BalanceLayer::Previous => CoreLayer::Previous,
        BalanceLayer::Gain => CoreLayer::Gain,
        BalanceLayer::Overdraft => CoreLayer::Overdraft,
        BalanceLayer::Zero => CoreLayer::Zero,
        BalanceLayer::Grid => CoreLayer::Grid,
    }
}

/// The charts draw from what the last load stored.
pub fn connect(state: &Rc<State>, store: &FinanceStore<'_>) {
    store.on_sparkline_path({
        let state = state.clone();
        move |kind, w, h, _| {
            let i = match kind {
                SummaryCardKind::Total => 0,
                SummaryCardKind::Balance => 1,
                SummaryCardKind::Income => 2,
                SummaryCardKind::Expense => 3,
            };
            finances::sparkline_path(&state.charts.borrow().cards[i], w, h).into()
        }
    });
    store.on_balance_path({
        let state = state.clone();
        move |layer, w, h, _| {
            let charts = state.charts.borrow();
            charts.balance.layer(core_layer(layer), w, h).into()
        }
    });
    store.on_donut_path({
        let state = state.clone();
        move |i, w, h, _| {
            let index = usize::try_from(i).ok();
            finances::donut_path(&state.charts.borrow().shares, w, h, index).into()
        }
    });
}

impl State {
    /// Loads the shown month's Summary.
    pub fn load_summary(&self, w: &AppWindow) -> Result<(), String> {
        let (month, today) = (self.shown.get(), Local::now().date_naive());
        let c = self.main()?;
        let style = style();
        let money = |v: i64| finances::format_amount(v, c, style);
        let (store, s) = (w.global::<FinanceStore>(), w.global::<Strings>());
        let err = |e: finances::FinanceError| e.to_string();

        let cards = finances::month_cards(&self.conn, month, today).map_err(err)?;
        store.set_summary_total(card_row(&cards.total, c, &s, true));
        store.set_summary_balance(card_row(&cards.balance, c, &s, false));
        store.set_summary_income(card_row(&cards.income, c, &s, false));
        store.set_summary_expense(card_row(&cards.expense, c, &s, false));

        let chart = cards.balance_chart();
        store.set_balance_net(money(cards.balance.value).into());
        store.set_balance_income(money(cards.income.value).into());
        store.set_balance_expense(money(cards.expense.value).into());
        store.set_balance_previous(money(cards.previous.value).into());
        store.set_balance_last_day(i32::try_from(cards.shown).unwrap_or(i32::MAX));
        store.set_balance_zero(chart.zero_pos());
        let y = chart.y_labels().into_iter().map(|l| AxisLabelRow {
            text: finances::compact_amount(l.value, c, style).into(),
            day: 0,
            pos: l.pos,
        });
        store.set_balance_y_labels(ModelRc::new(VecModel::from(y.collect::<Vec<_>>())));
        let x = chart.x_labels().into_iter().map(|l| AxisLabelRow {
            text: Default::default(),
            day: i32::try_from(l.value).unwrap_or(0),
            pos: l.pos,
        });
        store.set_balance_x_labels(ModelRc::new(VecModel::from(x.collect::<Vec<_>>())));

        let kind = core_kind(store.get_summary_kind());
        let donut = finances::category_breakdown(&self.conn, month, kind).map_err(err)?;
        store.set_summary_kind_total(money(donut.total).into());
        let slices = donut
            .slices
            .iter()
            .map(|d| DonutSliceRow {
                name: match (&d.name, d.others) {
                    (_, true) => s.get_others(),
                    (Some(n), _) => n.as_str().into(),
                    (None, _) => s.get_no_category(),
                },
                percent: format!("{}%", d.percent).into(),
            })
            .collect::<Vec<_>>();
        store.set_summary_slices(ModelRc::new(VecModel::from(slices)));

        let filter = core_filter(store.get_summary_filter());
        let recent = finances::recent_movements(&self.conn, month, filter, LISTED)
            .map_err(err)?
            .into_iter()
            .map(|m| row(m, c))
            .collect::<Result<Vec<_>, _>>()?;
        store.set_summary_recent(ModelRc::new(VecModel::from(recent)));

        let filter = core_upcoming(store.get_summary_upcoming_filter());
        let upcoming =
            finances::upcoming_payments(&self.conn, today, filter, LISTED).map_err(err)?;
        let bars = upcoming.months.iter().map(|m| UpcomingBarRow {
            month: m.month.month() as i32,
            total: money(m.total).into(),
            share: m.share,
        });
        store.set_summary_bars(ModelRc::new(VecModel::from(bars.collect::<Vec<_>>())));
        let items = upcoming.items.into_iter().map(|i| UpcomingRow {
            description: match (i.bill, i.description.is_empty()) {
                (true, _) => s.invoke_bill_title(i.description.into()),
                (false, true) => s.invoke_tx_kind(TxKind::Expense),
                (false, false) => i.description.into(),
            },
            date: ui_date(i.date),
            amount: finances::format_in(i.amount, i.currency, c, style).into(),
        });
        store.set_summary_upcoming(ModelRc::new(VecModel::from(items.collect::<Vec<_>>())));

        *self.charts.borrow_mut() = Charts {
            cards: [
                cards.total.series,
                cards.balance.series,
                cards.income.series,
                cards.expense.series,
            ],
            balance: chart,
            shares: donut.slices.iter().map(|d| d.share).collect(),
        };
        store.set_chart_version(store.get_chart_version().wrapping_add(1));
        store.set_month(ui_date(month));
        Ok(())
    }
}
