-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- A named sub-balance inside a plain account: it lowers what is available, not the balance.
CREATE TABLE finance_reserve (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    account_id INTEGER NOT NULL REFERENCES finance_account (id),
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX finance_reserve_account ON finance_reserve (account_id);

CREATE TABLE finance_reserve_move (
    id INTEGER PRIMARY KEY,
    reserve_id INTEGER NOT NULL REFERENCES finance_reserve (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('deposit', 'withdraw')),
    amount INTEGER NOT NULL CHECK (amount > 0),
    date TEXT NOT NULL CHECK (date(date) IS date),
    description TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX finance_reserve_move_reserve ON finance_reserve_move (reserve_id);
