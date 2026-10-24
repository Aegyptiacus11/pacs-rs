-- For the REST API: each resource's public ID (pacs_core::public_ids) and its main tags
-- (pacs_core::main_tags) as a JSON object of raw strings; a study also keeps its
-- patient's. NULL in rows indexed before this migration.

ALTER TABLE studies ADD COLUMN public_id TEXT;
ALTER TABLE studies ADD COLUMN patient_public_id TEXT;
ALTER TABLE studies ADD COLUMN main_tags TEXT;
ALTER TABLE studies ADD COLUMN patient_main_tags TEXT;
CREATE UNIQUE INDEX studies_by_public_id ON studies (public_id);

ALTER TABLE series ADD COLUMN public_id TEXT;
ALTER TABLE series ADD COLUMN main_tags TEXT;
CREATE UNIQUE INDEX series_by_public_id ON series (public_id);

ALTER TABLE instances ADD COLUMN public_id TEXT;
ALTER TABLE instances ADD COLUMN main_tags TEXT;
CREATE UNIQUE INDEX instances_by_public_id ON instances (public_id);
