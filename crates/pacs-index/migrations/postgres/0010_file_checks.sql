-- The SHA-256 of each stored file, taken when it is stored, and its last check against the
-- file in the store: when, and what was wrong (NULL when it matched). Files stored before
-- this have no hash until their first check, which takes it.

ALTER TABLE instances ADD COLUMN sha256 TEXT;
ALTER TABLE instances ADD COLUMN checked_ms BIGINT;
ALTER TABLE instances ADD COLUMN check_failure TEXT;

CREATE INDEX instances_by_checked ON instances (checked_ms);
