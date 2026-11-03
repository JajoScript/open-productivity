-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

ALTER TABLE task ADD COLUMN due_date TEXT CHECK (due_date IS NULL OR date(due_date) IS due_date);

CREATE INDEX task_due_date ON task (due_date);
