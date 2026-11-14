-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

CREATE TABLE finance_scheduled (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('expense', 'income')),
    description TEXT NOT NULL DEFAULT '',
    account_id INTEGER NOT NULL REFERENCES finance_account (id),
    category_id INTEGER REFERENCES finance_category (id),
    amount INTEGER NOT NULL CHECK (amount > 0),
    -- The day of the month that monthly and yearly dates keep.
    start_date TEXT NOT NULL CHECK (date(start_date) IS start_date),
    next_date TEXT NOT NULL CHECK (date(next_date) IS next_date),
    form TEXT NOT NULL CHECK (form IN ('once', 'recurring', 'installments')),
    frequency TEXT CHECK (frequency IN ('weekly', 'monthly', 'yearly')),
    installments_total INTEGER,
    installments_paid INTEGER NOT NULL DEFAULT 0 CHECK (installments_paid >= 0),
    done INTEGER NOT NULL DEFAULT 0 CHECK (done IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    CHECK ((form = 'recurring') = (frequency IS NOT NULL)),
    CHECK ((form = 'installments') = (installments_total IS NOT NULL)),
    CHECK (installments_total IS NULL OR installments_total >= 2),
    CHECK (installments_paid <= coalesce(installments_total, installments_paid))
) STRICT;

CREATE INDEX finance_scheduled_account ON finance_scheduled (account_id);
CREATE INDEX finance_scheduled_category ON finance_scheduled (category_id);

ALTER TABLE finance_transaction
    ADD COLUMN scheduled_id INTEGER REFERENCES finance_scheduled (id) ON DELETE SET NULL;
ALTER TABLE finance_transaction ADD COLUMN installment INTEGER CHECK (installment > 0);

CREATE INDEX finance_transaction_scheduled ON finance_transaction (scheduled_id);
