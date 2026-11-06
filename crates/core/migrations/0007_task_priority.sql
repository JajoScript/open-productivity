-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- Eisenhower matrix: two marks the user sets by hand.
ALTER TABLE task ADD COLUMN urgent INTEGER NOT NULL DEFAULT 0 CHECK (urgent IN (0, 1));
ALTER TABLE task ADD COLUMN important INTEGER NOT NULL DEFAULT 0 CHECK (important IN (0, 1));
