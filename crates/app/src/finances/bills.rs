// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 JajoScript

//! The card bills of the "To pay" tab.

use open_productivity_core::chrono::Local;
use open_productivity_core::finances::{self, Bill};

use super::state::State;
use super::style;
use crate::BillRow;
use crate::dates::ui_date;
use crate::tasks::ui_id;

impl State {
    fn bill_row(&self, b: Bill) -> Result<BillRow, String> {
        let (main, style) = (self.main()?, style());
        let source = finances::bill_payment_source(&self.conn, b.card)
            .map_err(|e| e.to_string())?
            .map(|id| finances::get_account(&self.conn, id))
            .transpose()
            .map_err(|e| e.to_string())?;
        Ok(BillRow {
            card_id: ui_id(b.card.0)?,
            card: b.card_name.into(),
            currency: b.currency.code.into(),
            closing: ui_date(b.closing),
            due: ui_date(b.due),
            pending: finances::format_in(b.pending, b.currency, main, style).into(),
            input: finances::format_input(b.pending, b.currency, style).into(),
            overdue: b.overdue,
            source_id: source.as_ref().map_or(Ok(0), |a| ui_id(a.id.0))?,
            same_currency: source
                .as_ref()
                .is_none_or(|a| a.currency.code == b.currency.code),
            source_currency: source.map_or_else(Default::default, |a| a.currency.code.into()),
        })
    }

    /// Loads the card bills due in the shown month.
    pub fn load_bills(&self) -> Result<(), String> {
        let rows =
            finances::bills_due_in_month(&self.conn, self.shown.get(), Local::now().date_naive())
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|b| self.bill_row(b))
                .collect::<Result<Vec<_>, _>>()?;
        self.bill_rows.set_vec(rows);
        Ok(())
    }
}
