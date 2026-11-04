-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- A free-form multi-line note per task; empty when there is none.
ALTER TABLE task ADD COLUMN note TEXT NOT NULL DEFAULT '';
