-- Deleted resources kept for a while (PACS_TRASH_DAYS): the index forgets them at once, so
-- nothing serves or lists them, and their files wait under trash/ in the store until the
-- trash is emptied or they are restored.

CREATE TABLE trash (
    id         INTEGER PRIMARY KEY,
    deleted_ms INTEGER NOT NULL,
    -- what DELETE named: Patient, Study, Series or Instance, and its public ID
    level      TEXT NOT NULL,
    public_id  TEXT NOT NULL,
    -- JSON list, a study each: its UID, patient, project, and the viewer's annotations when
    -- the whole study went
    studies    TEXT NOT NULL
);

CREATE TABLE trash_files (
    -- where the file waits, and where it was
    trash_key  TEXT PRIMARY KEY,
    trash_id   INTEGER NOT NULL REFERENCES trash (id) ON DELETE CASCADE,
    object_key TEXT NOT NULL
);

CREATE INDEX trash_files_by_trash ON trash_files (trash_id);
CREATE INDEX trash_by_deleted ON trash (deleted_ms);
