-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

CREATE TABLE list (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL CHECK (length(trim(name)) > 0)
) STRICT;

CREATE UNIQUE INDEX list_name ON list (name COLLATE NOCASE);

ALTER TABLE task ADD COLUMN list_id INTEGER REFERENCES list (id) ON DELETE SET NULL;

CREATE INDEX task_list_id ON task (list_id);
