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

## The store and the index compared

Checksums find a file that changed. A file the index does not know, left by a crash between
storing a file and indexing it, or an index entry whose file is gone, is found by comparing
the two:

```bash
pacs-server --consistency            # list what differs; exit 1 if anything does
pacs-server --consistency --repair   # and repair it
```

It runs with the server's settings, and can run beside a running server: files written in
the last ten minutes are left out, since an instance being stored has its file a moment
before its index entry. `--repair`:

- indexes again a file no entry names, when it reads as DICOM, as if it had just been
  received. A file that does not, or that is a copy of an instance indexed under another
  key, is listed and left where it is: repair never deletes a file;
- removes an entry whose file is gone, once the store confirms it, with its series and
  study when nothing else is left in them, so that searches stop offering images that
  cannot be served. The [audit trail](audit.md) records it as a deletion by the server
  (Study Deleted, or Instances Accessed with the action Delete when the study keeps other
  instances). To keep the entry, put the file back from a [backup](backup.md) first.

The comparison holds every index entry's key in memory, about 200 bytes an instance. It
refuses to run when `PACS_QUARANTINE` is inside `PACS_STORAGE`: quarantined files are kept
identified and never indexed, and would look like files the index lost.
