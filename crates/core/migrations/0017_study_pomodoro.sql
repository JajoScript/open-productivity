-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- Pomodoro state of the running session. `preset` is the focus minutes; NULL means Flowtime.
ALTER TABLE study_active ADD COLUMN preset INTEGER CHECK (preset IN (15, 25, 50));
ALTER TABLE study_active ADD COLUMN on_break INTEGER NOT NULL DEFAULT 0 CHECK (on_break IN (0, 1));
-- Seconds spent in the current phase.
ALTER TABLE study_active ADD COLUMN phase_secs INTEGER NOT NULL DEFAULT 0 CHECK (phase_secs >= 0);
ALTER TABLE study_active ADD COLUMN pomodoros INTEGER NOT NULL DEFAULT 0 CHECK (pomodoros >= 0);
