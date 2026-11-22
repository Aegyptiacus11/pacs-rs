-- The way back from a project's de-identified copy to the original, for write-back: a
-- result made on the copy refers to replaced UIDs and a pseudonym, and goes to the
-- hospital with the originals. Identifying data: no route serves it, and backups hold it.

-- Each UID a project's de-identification replaced, with the one it replaced.
CREATE TABLE project_uids (
    project  TEXT NOT NULL,
    replaced TEXT NOT NULL,
    original TEXT NOT NULL,
    PRIMARY KEY (project, replaced)
);

-- The original patient and study of each de-identified study: their main tags as JSON
-- objects of keyword to value, as the instance carried them.
CREATE TABLE project_studies (
    project           TEXT NOT NULL,
    replaced_study    TEXT NOT NULL,
    patient_main_tags TEXT NOT NULL,
    study_main_tags   TEXT NOT NULL,
    PRIMARY KEY (project, replaced_study)
);
