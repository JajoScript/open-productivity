-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

CREATE TABLE subject (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE TABLE study_session (
    id INTEGER PRIMARY KEY,
    subject_id INTEGER NOT NULL REFERENCES subject (id),
    -- YYYY-MM-DD, local day the session started.
    date TEXT NOT NULL CHECK (date(date) IS date),
    focus_minutes INTEGER NOT NULL CHECK (focus_minutes > 0),
    description TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX study_session_subject ON study_session (subject_id);
