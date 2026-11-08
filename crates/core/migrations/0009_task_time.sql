-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- Optional local times, 'HH:MM' without time zone. The rules (a time needs a due date, the end
-- is after the start) live in core::tasks.
ALTER TABLE task ADD COLUMN due_time TEXT;
ALTER TABLE task ADD COLUMN end_time TEXT;
