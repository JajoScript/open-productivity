-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- A goal is numeric when it has a target; its progress entries add up toward it.
ALTER TABLE goal ADD COLUMN target_count INTEGER CHECK (target_count IS NULL OR target_count > 0);
ALTER TABLE goal ADD COLUMN unit TEXT;

CREATE TABLE goal_entry (
    id INTEGER PRIMARY KEY,
    goal_id INTEGER NOT NULL REFERENCES goal (id),
    date TEXT NOT NULL CHECK (date(date) IS date),
    value INTEGER NOT NULL CHECK (value > 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX goal_entry_goal_date ON goal_entry (goal_id, date);
