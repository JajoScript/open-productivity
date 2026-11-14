-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

CREATE TABLE finance_transaction (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('expense', 'income')),
    account_id INTEGER NOT NULL REFERENCES finance_account (id),
    amount INTEGER NOT NULL CHECK (amount > 0),
    date TEXT NOT NULL CHECK (date(date) IS date),
    description TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX finance_transaction_date ON finance_transaction (date);
CREATE INDEX finance_transaction_account ON finance_transaction (account_id);
