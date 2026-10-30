-- The measurements drawn in the web interface's viewer, kept per study so they come back when
-- it is opened again, on any browser: the study's list as the interface wrote it (JSON), and
-- when it was last saved. Deleting the study deletes them.

CREATE TABLE ui_annotations (
    study_instance_uid TEXT PRIMARY KEY,
    body               TEXT NOT NULL,
    updated_ms         INTEGER NOT NULL
);
