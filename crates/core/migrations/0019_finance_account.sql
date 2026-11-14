-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

CREATE TABLE finance_account (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    kind TEXT NOT NULL CHECK (kind IN ('checking', 'demand', 'savings', 'cash')),
    bank TEXT NOT NULL DEFAULT '',
    currency TEXT NOT NULL CHECK (length(currency) = 3),
    initial_balance INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;
