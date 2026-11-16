-- The project a study came in through: the name of the de-identification profile of the AE
-- title it was sent to (docs/de-identification.md). NULL for a study stored as received.
-- Set by the study's first instance and never changed: Index::record refuses an instance
-- that comes for another project than its study's.

ALTER TABLE studies ADD COLUMN project TEXT;

CREATE INDEX studies_by_project ON studies (project);
