-- The web interface's hanging protocols, shared by everyone who uses this server: one row
-- each, the protocol as the interface wrote it (JSON), and when it was last saved.

CREATE TABLE ui_protocols (
    id         TEXT PRIMARY KEY,
    body       TEXT NOT NULL,
    updated_ms BIGINT NOT NULL
);
