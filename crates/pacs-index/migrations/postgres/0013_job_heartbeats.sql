-- When a running job last showed it was alive: the server running it writes this four times a
-- second. A job silent for long is a stopped server's, which any server may take up.

ALTER TABLE jobs ADD COLUMN heartbeat_ms BIGINT;
