-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- The kind CHECK must accept 'credit', so the table is rebuilt. Other tables reference it and
-- migrations run inside a transaction, where foreign_keys cannot be switched off: the
-- violations of the DROP are deferred to the commit, when the new table has the same ids.
PRAGMA defer_foreign_keys = ON;

CREATE TEMP TABLE finance_account_old AS SELECT * FROM finance_account;
DROP TABLE finance_account;

CREATE TABLE finance_account (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    kind TEXT NOT NULL CHECK (kind IN ('checking', 'demand', 'savings', 'cash', 'credit')),
    bank TEXT NOT NULL DEFAULT '',
    currency TEXT NOT NULL CHECK (length(currency) = 3),
    initial_balance INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    credit_limit INTEGER CHECK (credit_limit > 0),
    closing_day INTEGER CHECK (closing_day BETWEEN 1 AND 31),
    due_day INTEGER CHECK (due_day BETWEEN 1 AND 31),
    CHECK ((kind = 'credit') = (credit_limit IS NOT NULL
        AND closing_day IS NOT NULL AND due_day IS NOT NULL)),
    CHECK (kind = 'credit' OR (credit_limit IS NULL
        AND closing_day IS NULL AND due_day IS NULL))
) STRICT;

INSERT INTO finance_account
    (id, name, kind, bank, currency, initial_balance, created_at, archived)
SELECT id, name, kind, bank, currency, initial_balance, created_at, archived
FROM finance_account_old;

DROP TABLE finance_account_old;
