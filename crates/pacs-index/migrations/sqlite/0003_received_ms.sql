-- When each instance arrived, in milliseconds since the Unix epoch, written by pacs-rs
-- itself so that SQLite and Postgres keep it alike. `received_at` stays for older rows.

ALTER TABLE instances ADD COLUMN received_ms INTEGER;
UPDATE instances
   SET received_ms = CAST((julianday(received_at) - 2440587.5) * 86400000 AS INTEGER);
