-- SPDX-License-Identifier: GPL-3.0-or-later
-- Copyright (C) 2026 JajoScript

-- Minutes per day reads sessions by date range.
CREATE INDEX study_session_date ON study_session (date);
