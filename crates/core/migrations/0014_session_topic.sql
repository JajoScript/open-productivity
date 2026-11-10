-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

ALTER TABLE study_session ADD COLUMN topic_id INTEGER REFERENCES task (id) ON DELETE SET NULL;

CREATE INDEX study_session_topic ON study_session (topic_id);
