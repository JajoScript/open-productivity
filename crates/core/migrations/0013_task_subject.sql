-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

ALTER TABLE task ADD COLUMN subject_id INTEGER REFERENCES subject (id);

CREATE INDEX task_subject ON task (subject_id);
