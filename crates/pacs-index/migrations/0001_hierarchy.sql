-- The study / series / instance hierarchy. `attributes` columns hold DICOM JSON of the
-- level's QIDO attributes (pacs_core::levels); `metadata` holds an instance's whole data
-- set without pixel data, for WADO-RS metadata. Scalar columns exist for matching.

CREATE TABLE studies (
    study_instance_uid TEXT PRIMARY KEY NOT NULL,
    patient_id         TEXT,
    patient_name       TEXT,
    study_date         TEXT,
    accession_number   TEXT,
    attributes         TEXT NOT NULL
) STRICT;

CREATE INDEX studies_by_patient_id ON studies (patient_id);
CREATE INDEX studies_by_study_date ON studies (study_date);
CREATE INDEX studies_by_accession_number ON studies (accession_number);

CREATE TABLE series (
    series_instance_uid TEXT PRIMARY KEY NOT NULL,
    study_instance_uid  TEXT NOT NULL REFERENCES studies ON DELETE CASCADE,
    modality            TEXT,
    series_number       INTEGER,
    attributes          TEXT NOT NULL
) STRICT;

CREATE INDEX series_by_study ON series (study_instance_uid);

CREATE TABLE instances (
    sop_instance_uid    TEXT PRIMARY KEY NOT NULL,
    series_instance_uid TEXT NOT NULL REFERENCES series ON DELETE CASCADE,
    sop_class_uid       TEXT NOT NULL,
    instance_number     INTEGER,
    transfer_syntax_uid TEXT NOT NULL,
    object_key          TEXT NOT NULL,
    size_bytes          INTEGER NOT NULL,
    attributes          TEXT NOT NULL,
    metadata            TEXT NOT NULL,
    received_at         TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
) STRICT;

CREATE INDEX instances_by_series ON instances (series_instance_uid, instance_number);
