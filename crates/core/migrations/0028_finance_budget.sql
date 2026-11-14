-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

CREATE TABLE finance_budget_bucket (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    position INTEGER NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- The percentage of a bucket from `month` (its first day) on, until a later month has shares.
CREATE TABLE finance_budget_share (
    month TEXT NOT NULL CHECK (date(month) = month AND strftime('%d', month) = '01'),
    bucket_id INTEGER NOT NULL REFERENCES finance_budget_bucket (id) ON DELETE CASCADE,
    percent INTEGER NOT NULL CHECK (percent BETWEEN 0 AND 100),
    PRIMARY KEY (month, bucket_id)
) STRICT;

ALTER TABLE finance_category
    ADD COLUMN bucket_id INTEGER REFERENCES finance_budget_bucket (id) ON DELETE SET NULL;
