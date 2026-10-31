INSERT INTO schema_migrations(version) VALUES (14);

-- Production workflow, stage 5: semester calendar and exact deadlines.
-- One semester per workspace database. Week dates are derived from it; without a
-- semester they stay unknown instead of being invented.
CREATE TABLE semester_settings (
    singleton INTEGER PRIMARY KEY NOT NULL DEFAULT 1 CHECK (singleton = 1),
    start_date TEXT NOT NULL CHECK (date(start_date) IS start_date),
    utc_offset_minutes INTEGER NOT NULL
        CHECK (utc_offset_minutes BETWEEN -720 AND 840 AND utc_offset_minutes % 15 = 0),
    week_count INTEGER NOT NULL CHECK (week_count BETWEEN 1 AND 60),
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    updated_at TEXT NOT NULL
) STRICT;

-- Where the dates of a week come from. Dates written before v14 were set by routing
-- relative to "now" and are only approximate: they are marked legacy.
ALTER TABLE weeks ADD COLUMN dates_source TEXT NOT NULL DEFAULT 'unknown'
    CHECK (dates_source IN ('unknown', 'semester', 'legacy'));
UPDATE weeks SET dates_source = 'legacy' WHERE length(trim(starts_at)) > 0;

-- Where an exact deadline comes from. NULL with a deadline = written before v14 (legacy).
ALTER TABLE homeworks ADD COLUMN deadline_source TEXT
    CHECK (deadline_source IS NULL OR deadline_source IN ('manual', 'legacy', 'lms'));
