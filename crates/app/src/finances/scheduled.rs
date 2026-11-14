// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The "To pay" tab: the month's due payments and the scheduled payment callbacks.

use std::rc::Rc;

use open_productivity_core::calendar;
use open_productivity_core::chrono::{Datelike, Local, NaiveDate};
use open_productivity_core::finances::{
    self, AccountId, CategoryId, Currency, DayMark, DueKind, DueRow, FinanceError,
    ScheduleForm as Form, ScheduledId, TxKind as CoreKind,
};
use open_productivity_core::recurrence::Frequency;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::state::State;
use super::style;
use super::transactions::{core_kind, ui_kind};
use crate::dates::ui_date;
use crate::tasks::ui_id;
use crate::{AppWindow, BiggestRow, DueCell, DueMark, FinanceStore, ScheduleForm, ScheduledRow};

fn due_row(d: DueRow, main: &Currency) -> Result<ScheduledRow, String> {
    let (s, style) = (d.scheduled, style());
    let (form, number, total) = match d.kind {
        DueKind::Once => (ScheduleForm::Once, 0, 0),
        DueKind::Recurring(Frequency::Weekly) => (ScheduleForm::Weekly, 0, 0),
        DueKind::Recurring(Frequency::Yearly) => (ScheduleForm::Yearly, 0, 0),
        DueKind::Recurring(_) => (ScheduleForm::Monthly, 0, 0),
        DueKind::Installment { number, total } => {
            (ScheduleForm::Installments, ui_id(number)?, ui_id(total)?)
        }
    };
    let signed = match s.kind {
        CoreKind::Expense => -s.amount,
        CoreKind::Income => s.amount,
    };
    Ok(ScheduledRow {
        id: ui_id(s.id.0)?,
        kind: ui_kind(s.kind),
        account_id: ui_id(s.account_id.0)?,
        account: s.account_name.into(),
        currency: s.currency.code.into(),
        has_category: s.category.is_some(),
        category_id: s.category.map_or(Ok(0), |c| ui_id(c.0))?,
        description: s.description.into(),
        form,
        number,
        total,
        date: ui_date(d.date),
        next_date: ui_date(s.next_date),
        amount: finances::format_in(signed, s.currency, main, style).into(),
        input: finances::format_input(s.amount, s.currency, style).into(),
        is_next: d.is_next,
        overdue: d.overdue,
    })
}

/// The frequency a recurring form stands for; `None` for the other forms.
fn frequency(f: ScheduleForm) -> Option<Frequency> {
    match f {
        ScheduleForm::Weekly => Some(Frequency::Weekly),
        ScheduleForm::Monthly => Some(Frequency::Monthly),
        ScheduleForm::Yearly => Some(Frequency::Yearly),
        _ => None,
    }
}

impl State {
    /// Loads the due payments of the shown month into the list and the title.
    pub fn load_due(&self, w: &AppWindow) -> Result<(), String> {
        let month = self.shown.get();
        let main = self.main()?;
        let rows = finances::due_in_month(&self.conn, month, Local::now().date_naive())
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|d| due_row(d, main))
            .collect::<Result<Vec<_>, _>>()?;
        self.due_rows.set_vec(rows);
        self.load_bills()?;
        self.load_due_extras(w, month, main)?;
        w.global::<FinanceStore>().set_month(ui_date(month));
        Ok(())
    }
}

/// How many payments the "Biggest to pay" bars show.
const BIGGEST: usize = 4;

impl State {
    /// The calendar marks and the biggest payments of the shown month.
    fn load_due_extras(
        &self,
        w: &AppWindow,
        month: NaiveDate,
        main: &Currency,
    ) -> Result<(), String> {
        let (today, style) = (Local::now().date_naive(), style());
        let err = |e: FinanceError| e.to_string();
        let marks = finances::due_days(&self.conn, month, today).map_err(err)?;
        let grid = calendar::month_grid(month.year(), month.month(), 0, today)
            .ok_or_else(|| "invalid month".to_string())?;
        let cells = grid.cells.iter().map(|c| {
            let day = c.unwrap_or(0);
            let mark = marks
                .iter()
                .find(|(d, _)| c.is_some() && d.day() == day)
                .map_or(DueMark::None, |(_, m)| match m {
                    DayMark::Due => DueMark::Due,
                    DayMark::Overdue => DueMark::Overdue,
                });
            DueCell {
                day: day as i32,
                mark,
                today: c.is_some() && grid.today == Some(day),
            }
        });
        let biggest = finances::biggest_due(&self.conn, month, today, BIGGEST)
            .map_err(err)?
            .into_iter()
            .map(|b| BiggestRow {
                description: b.description.into(),
                amount: finances::format_amount(b.amount, main, style).into(),
                share: b.share,
            });
        let store = w.global::<FinanceStore>();
        store.set_due_cells(ModelRc::new(VecModel::from(cells.collect::<Vec<_>>())));
        store.set_due_biggest(ModelRc::new(VecModel::from(biggest.collect::<Vec<_>>())));
        Ok(())
    }
}

pub fn connect(state: &Rc<State>, store: &FinanceStore<'_>) {
    store.on_create_scheduled({
        let state = state.clone();
        move |kind, account, has_category, category, amount, description, date, form, count| {
            // Text that is not a number is not valid installments.
            let form = match form {
                ScheduleForm::Once => Ok(Form::Once),
                ScheduleForm::Installments => count
                    .trim()
                    .parse()
                    .map(Form::Installments)
                    .map_err(|_| FinanceError::InvalidSchedule),
                f => Ok(Form::Recurring(frequency(f).unwrap_or(Frequency::Monthly))),
            };
            state.with_date(date, |d| {
                finances::create_scheduled(
                    &state.conn,
                    core_kind(kind),
                    &description,
                    AccountId(account.into()),
                    &amount,
                    has_category.then_some(CategoryId(category.into())),
                    d,
                    form?,
                )
                .map(|_| ())
            })
        }
    });

    store.on_update_scheduled({
        let state = state.clone();
        move |id, account, has_category, category, amount, description, date, form| {
            state.with_date(date, |d| {
                finances::update_scheduled(
                    &state.conn,
                    ScheduledId(id.into()),
                    &description,
                    &amount,
                    AccountId(account.into()),
                    has_category.then_some(CategoryId(category.into())),
                    d,
                    frequency(form),
                )
            })
        }
    });

    store.on_confirm_scheduled({
        let state = state.clone();
        move |id, date, amount| -> SharedString {
            state.with_date(date, |d| {
                finances::confirm_scheduled(&state.conn, ScheduledId(id.into()), d, &amount)
            })
        }
    });

    store.on_skip_scheduled({
        let state = state.clone();
        move |id| {
            state.finish(finances::skip_scheduled(
                &state.conn,
                ScheduledId(id.into()),
            ))
        }
    });

    store.on_delete_scheduled({
        let state = state.clone();
        move |id| {
            state.finish(finances::delete_scheduled(
                &state.conn,
                ScheduledId(id.into()),
            ))
        }
    });
}
