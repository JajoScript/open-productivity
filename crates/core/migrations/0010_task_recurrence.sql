-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- "Repeat": completing the task creates the next one and the rule moves to it. The rule lives
-- in core::tasks; anchor_day is the day of the month the series started on.
CREATE TABLE task_recurrence (
    task_id INTEGER NOT NULL UNIQUE REFERENCES task (id) ON DELETE CASCADE,
    frequency TEXT NOT NULL CHECK (frequency IN ('daily', 'weekly', 'monthly', 'yearly')),
    interval INTEGER NOT NULL CHECK (interval >= 1),
    anchor_day INTEGER NOT NULL CHECK (anchor_day BETWEEN 1 AND 31)
) STRICT;
