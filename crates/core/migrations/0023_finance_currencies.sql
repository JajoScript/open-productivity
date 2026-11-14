-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- One manual rate per non-main currency: what 1 major unit is worth in the main currency, with
-- 6 fixed decimals (1 USD = 987.24 CLP is 987240000).
CREATE TABLE finance_rate (
    currency TEXT PRIMARY KEY CHECK (length(currency) = 3),
    rate INTEGER NOT NULL CHECK (rate > 0)
) STRICT;

-- The amount that arrives in the destination account of a transfer between different currencies.
ALTER TABLE finance_transaction ADD COLUMN to_amount INTEGER CHECK (to_amount > 0);
