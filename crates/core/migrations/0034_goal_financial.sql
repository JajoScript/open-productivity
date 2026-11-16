-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- A goal is financial when it has a target amount, in the currency of its reserve's account.
ALTER TABLE goal ADD COLUMN target_amount INTEGER CHECK (target_amount IS NULL OR target_amount > 0);

-- The one reserve of a financial goal; its balance is the goal's progress.
ALTER TABLE finance_reserve ADD COLUMN goal_id INTEGER REFERENCES goal (id);

CREATE UNIQUE INDEX finance_reserve_goal ON finance_reserve (goal_id) WHERE goal_id IS NOT NULL;
