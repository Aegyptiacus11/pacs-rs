-- The studies of projects that took in a result (an instance made on a de-identified copy)
-- since their results were last sent back: written with the instance, so that a result
-- stored is written back even when the server stops before it is. A study quiet for
-- PACS_FORWARD_AFTER is taken out and its results sent to the project's write-back
-- destination.

CREATE TABLE write_back_due (
    study_instance_uid TEXT PRIMARY KEY,
    -- when its latest result was recorded
    touched_ms         BIGINT NOT NULL
);
