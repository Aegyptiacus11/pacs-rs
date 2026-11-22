# Checking stored files

pacs-rs takes the SHA-256 of every file as it stores it, keeps it in the index with the
file's size, and reads files back to compare. A file changed on disk or in the bucket, cut
short, or gone is found, even when nobody asks for it.

- **On a schedule.** In the background, one file at a time, the server checks again every
  file whose last check is older than `PACS_CHECK_FILES_DAYS` (30 days; `0` turns it off).
  Files stored by a pacs-rs older than 0.13 have no hash yet: they are checked first, and
  their first check takes it.
- **On request.** `POST /tools/check-files` with `{"StudyInstanceUID": "..."}` checks that
  study before answering; with no body, the whole archive is checked in the background.
  `GET /tools/check-files` says how many files there are, how many were never checked, how
  the last whole-archive check went, and lists every file whose last check failed. Both are
  for a role with `users` (see [Login](login.md)), and are in the
  [API reference](api-reference.md#integrity).
- **From the command line.** `pacs-server --check-files`, with the server's settings,
  checks every file, prints the ones that failed and exits 1 if any did, 0 if not: for cron
  or a monitoring script. It can run beside a running server.

A failed file is logged at ERROR and listed until a check passes again; pacs-rs neither
deletes nor hides it. Put it back from a [backup](backup.md), or send the instance again,
which stores it with a new hash. A store that cannot be read (S3 down, a disk not mounted)
is an error of the check, not a bad file: nothing is marked, and the scheduled checks try
again ten minutes later.
