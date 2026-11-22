-- Labels on patients, studies, series and instances (the REST API's /labels): free words
-- a user or a tool puts on a resource, to group and find them. A resource deleted takes its
-- labels with it.

CREATE TABLE labels (
    -- the resource's public ID, and its level: Patient, Study, Series or Instance
    public_id TEXT NOT NULL,
    level     TEXT NOT NULL,
    label     TEXT NOT NULL,
    PRIMARY KEY (public_id, label)
);

CREATE INDEX labels_by_label ON labels (label);
CREATE INDEX studies_by_patient_public_id ON studies (patient_public_id);
