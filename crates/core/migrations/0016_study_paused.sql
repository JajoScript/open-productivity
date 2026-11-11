-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- A paused session counts no time until it is resumed.
ALTER TABLE study_active ADD COLUMN paused INTEGER NOT NULL DEFAULT 0 CHECK (paused IN (0, 1));
