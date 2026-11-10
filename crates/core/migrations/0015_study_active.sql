-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- The running Flowtime session; at most one row.
CREATE TABLE study_active (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    subject_id INTEGER NOT NULL REFERENCES subject (id),
    topic_id INTEGER REFERENCES task (id) ON DELETE SET NULL,
    -- YYYY-MM-DD, local day it started.
    started_on TEXT NOT NULL CHECK (date(started_on) IS started_on),
    counted_secs INTEGER NOT NULL DEFAULT 0 CHECK (counted_secs >= 0),
    -- UTC instant of the last time the clock was banked.
    last_seen TEXT NOT NULL
) STRICT;
