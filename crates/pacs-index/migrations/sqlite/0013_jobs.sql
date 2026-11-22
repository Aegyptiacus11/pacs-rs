-- The moves from remote archives that run in the background (/jobs), kept so that a
-- restart takes them up again, or says why it could not.

CREATE TABLE jobs (
    id               INTEGER PRIMARY KEY,
    -- C-MOVE
    kind             TEXT NOT NULL,
    remote           TEXT NOT NULL,
    destination      TEXT NOT NULL,
    -- JSON list of Study Instance UIDs
    studies          TEXT NOT NULL,
    -- Running, Success, Warning, Cancelled or Failure
    state            TEXT NOT NULL,
    -- JSON: the counts of the remote AE's last response
    progress         TEXT NOT NULL,
    -- JSON list of SOP Instance UIDs
    failed_instances TEXT NOT NULL,
    error            TEXT,
    -- 1 once a cancel is asked; the move reads it as it goes
    cancel_asked     INTEGER NOT NULL,
    -- how many times a start of the server took it up again
    restarts         INTEGER NOT NULL,
    started_ms       INTEGER NOT NULL,
    finished_ms      INTEGER
);

CREATE INDEX jobs_by_state ON jobs (state);
