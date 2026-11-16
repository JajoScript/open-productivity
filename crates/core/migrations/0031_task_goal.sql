-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

ALTER TABLE task ADD COLUMN goal_id INTEGER REFERENCES goal (id);

CREATE INDEX task_goal ON task (goal_id);
