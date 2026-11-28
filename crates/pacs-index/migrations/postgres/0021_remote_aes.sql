-- The remote AEs added in the web interface (PUT /modalities/{id}), beside those of
-- PACS_DIMSE_REMOTES, which stay in the settings: one row each, read when the server starts.

CREATE TABLE remote_aes (
    id           TEXT PRIMARY KEY,
    ae_title     TEXT NOT NULL,
    host         TEXT NOT NULL,
    port         BIGINT NOT NULL,
    -- 1: reached over TLS only
    secure       BIGINT NOT NULL,
    -- 1: studies may be sent to it (a C-MOVE destination)
    may_receive  BIGINT NOT NULL,
    updated_ms   BIGINT NOT NULL
);
