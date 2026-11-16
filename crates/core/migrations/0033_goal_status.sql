-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- A goal is completed once `completed_at` is set (never cleared); archiving hides it from the views.
ALTER TABLE goal ADD COLUMN completed_at TEXT;
ALTER TABLE goal ADD COLUMN archived INTEGER NOT NULL DEFAULT 0 CHECK (archived IN (0, 1));
