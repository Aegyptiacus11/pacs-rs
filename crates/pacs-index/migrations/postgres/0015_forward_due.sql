-- The studies that took in an instance since they were last forwarded by a rule
-- (PACS_FORWARD): written with the instance, so that what is stored is forwarded even when
-- the server stops before it is. A study quiet for PACS_FORWARD_AFTER is taken out and
-- forwarded.

CREATE TABLE forward_due (
    study_instance_uid TEXT PRIMARY KEY,
    -- when its latest instance was recorded
    touched_ms         BIGINT NOT NULL
);
