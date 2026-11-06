-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- Kanban: a pending task is in the icebox (0) or in progress (1); done_at still marks done.
ALTER TABLE task ADD COLUMN in_progress INTEGER NOT NULL DEFAULT 0 CHECK (in_progress IN (0, 1));
