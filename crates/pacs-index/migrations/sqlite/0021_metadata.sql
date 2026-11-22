-- User metadata on patients, studies, series and instances (the REST API's /metadata): a
-- text value under a name a user or a tool chooses. A resource deleted takes its metadata
-- with it.

CREATE TABLE metadata (
    -- the resource's public ID, and its level: Patient, Study, Series or Instance
    public_id TEXT NOT NULL,
    level     TEXT NOT NULL,
    name      TEXT NOT NULL,
    value     TEXT NOT NULL,
    PRIMARY KEY (public_id, name)
);
