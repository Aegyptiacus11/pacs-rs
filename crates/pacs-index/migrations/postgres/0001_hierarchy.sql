-- The study / series / instance hierarchy, as the SQLite migrations build it (see
-- ../sqlite): `attributes` and `main_tags` hold JSON text, `metadata` an instance's whole
-- data set without pixel data. JSON stays TEXT so it is sent exactly as stored.

CREATE TABLE studies (
    study_instance_uid TEXT PRIMARY KEY,
    patient_id         TEXT,
    patient_name       TEXT,
    study_date         TEXT,
    accession_number   TEXT,
    attributes         TEXT NOT NULL,
    public_id          TEXT UNIQUE,
    patient_public_id  TEXT,
    main_tags          TEXT,
    patient_main_tags  TEXT
);

CREATE INDEX studies_by_patient_id ON studies (patient_id);
CREATE INDEX studies_by_study_date ON studies (study_date);
CREATE INDEX studies_by_accession_number ON studies (accession_number);

CREATE TABLE series (
    series_instance_uid TEXT PRIMARY KEY,
    study_instance_uid  TEXT NOT NULL REFERENCES studies ON DELETE CASCADE,
    modality            TEXT,
    series_number       BIGINT,
    attributes          TEXT NOT NULL,
    public_id           TEXT UNIQUE,
    main_tags           TEXT
);

CREATE INDEX series_by_study ON series (study_instance_uid);

CREATE TABLE instances (
    sop_instance_uid    TEXT PRIMARY KEY,
    series_instance_uid TEXT NOT NULL REFERENCES series ON DELETE CASCADE,
    sop_class_uid       TEXT NOT NULL,
    instance_number     BIGINT,
    transfer_syntax_uid TEXT NOT NULL,
    object_key          TEXT NOT NULL,
    size_bytes          BIGINT NOT NULL,
    attributes          TEXT NOT NULL,
    metadata            TEXT NOT NULL,
    public_id           TEXT UNIQUE,
    main_tags           TEXT,
    received_ms         BIGINT NOT NULL
);

CREATE INDEX instances_by_series ON instances (series_instance_uid, instance_number);
