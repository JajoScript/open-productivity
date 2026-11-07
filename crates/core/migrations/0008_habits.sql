-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

CREATE TABLE habit (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    -- Bit 0 is Monday ... bit 6 is Sunday.
    weekdays INTEGER NOT NULL DEFAULT 127 CHECK (weekdays BETWEEN 1 AND 127),
    -- YYYY-MM-DD; the habit does not exist for earlier days.
    start_date TEXT NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

-- One row per habit and day it was done.
CREATE TABLE habit_log (
    habit_id INTEGER NOT NULL REFERENCES habit (id) ON DELETE CASCADE,
    date TEXT NOT NULL,
    PRIMARY KEY (habit_id, date)
) STRICT;
