-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

CREATE TABLE finance_transaction_new (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('expense', 'income', 'transfer')),
    account_id INTEGER NOT NULL REFERENCES finance_account (id),
    amount INTEGER NOT NULL CHECK (amount > 0),
    date TEXT NOT NULL CHECK (date(date) IS date),
    description TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    category_id INTEGER REFERENCES finance_category (id),
    to_account_id INTEGER REFERENCES finance_account (id),
    CHECK ((kind = 'transfer') = (to_account_id IS NOT NULL)),
    CHECK (to_account_id IS NOT account_id),
    CHECK (kind <> 'transfer' OR category_id IS NULL)
) STRICT;

INSERT INTO finance_transaction_new
    (id, kind, account_id, amount, date, description, created_at, category_id)
SELECT id, kind, account_id, amount, date, description, created_at, category_id
FROM finance_transaction;

DROP TABLE finance_transaction;
ALTER TABLE finance_transaction_new RENAME TO finance_transaction;

CREATE INDEX finance_transaction_date ON finance_transaction (date);
CREATE INDEX finance_transaction_account ON finance_transaction (account_id);
CREATE INDEX finance_transaction_category ON finance_transaction (category_id);
CREATE INDEX finance_transaction_to_account ON finance_transaction (to_account_id);

ALTER TABLE finance_account ADD COLUMN archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1));
