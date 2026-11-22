# Backing up and upgrading

An archive is two things: the files, and the index that says which file is which study.
A backup needs both, taken so that they agree. The steps on this page were run against a
server and its restored copy while the page was written.

## What to back up

| What | Where it is | Setting |
|---|---|---|
| The files | a directory, or an S3 bucket | `PACS_STORAGE` |
| The index | a SQLite file, or a Postgres database | `PACS_INDEX` |
| The audit trail, and its position file | a file, and `<file>.sent` beside it | `PACS_AUDIT_FILE` |
| The secrets | wherever you keep them | `PACS_AUTH_SECRET`, the projects' secrets |
| The settings | your `compose.yaml` or unit file | |

**The projects' secrets are part of the data.** A project's pseudonyms are made with its
secret: with another secret the same patient gets another pseudonym, and new studies no
longer join the old ones ([De-identification](de-identification.md)). A new
`PACS_AUTH_SECRET` costs much less: the access tokens already given out are refused, and
their holders get new ones ([Login](login.md)).

## The order: the index first, then the files

Back up the index, then the files, and do not delete studies in between.

A file that arrives after the index was saved is one more file in the backup that the saved
index does not name: harmless. A study deleted after the index was saved is named by the
saved index and missing from the files: a restore would list a study it cannot serve. The
safest backup is of a stopped server; the order above is for a running one.

## With SQLite

**Do not copy `index.sqlite` from under a running server.** The index is in write-ahead
mode, and what was written lately is in the `index.sqlite-wal` file beside it: a copy of
the main file alone opened as an empty index when this was tried.

A running server, with SQLite's own online backup, which reads a consistent state:

```bash
sqlite3 /data/index.sqlite ".backup '/backup/index.sqlite'"
cp -a /data/files /backup/files
```

The image has no shell, so this runs from the host or another container that mounts the
volume. Without `sqlite3`, the same from Python's standard library:

```bash
python3 -c "
import sqlite3
src = sqlite3.connect('file:/data/index.sqlite?mode=ro', uri=True)
dst = sqlite3.connect('/backup/index.sqlite')
src.backup(dst)"
```

A stopped server leaves one file, `index.sqlite`, and the whole directory can be copied:

```bash
docker compose stop
cp -a /data/. /backup/
docker compose start
```

## With Postgres

```bash
pg_dump -U pacs -d pacs | gzip > /backup/pacs-$(date +%Y%m%d).sql.gz
cp -a /data/files /backup/files
```

`pg_dump` reads one consistent state while the server runs.

## With S3

The bucket is the files. Back it up as your storage provides: versioning, replication to
another bucket, or a copy with a tool such as `rclone`. The order still holds: the index
first.

## Restoring

Into an empty place, never over the live one:

```bash
# SQLite: the saved index and files, as the server's data
cp -a /backup/index.sqlite /restore/index.sqlite
cp -a /backup/files /restore/files

# Postgres: into a new database
createdb -U pacs pacs_restored
gunzip -c /backup/pacs-20261117.sql.gz | psql -U pacs -q -d pacs_restored
```

Then start a server on the restored copy, on another port, and check it before trusting
it:

```bash
PACS_LISTEN=127.0.0.1:8081 PACS_INDEX=/restore/index.sqlite PACS_STORAGE=file:/restore/files pacs-server
curl http://127.0.0.1:8081/studies                       # the studies you expect
curl -o one.dcm http://127.0.0.1:8081/instances/<id>/file # and a file that comes back
```

**A backup that was never restored is a hope, not a backup.** Do this once when you set the
backup up, and again after an upgrade that changes the index.

## Upgrading

1. **Read the release's notes**: they say what changes without a setting.
2. **Back up the index.** An upgrade may change its layout, and that cannot be undone.
3. **Start the new version.** It brings the index up to its own layout by itself as it
   starts, before it serves anything.

```bash
PACS_IMAGE=ghcr.io/aegyptiacus11/pacs-rs:0.12.0 docker compose up -d --wait
```

Name the version, as above, rather than `latest`: the server then changes when you decide.

**There is no going back by starting the old version.** An older version that finds an
index changed by a newer one does not start, and says which change it does not know: it
does not touch the index. To go back, restore the index you saved in step 2, and start the
old version on it. What arrived since that backup is then in the files and not in the
index.

The files never change with an upgrade: an instance is kept as it was stored.

## Checking a backup routinely

- Count: `curl .../studies` on the restored copy against the live server.
- Read: fetch one instance's file from the restored copy.
- Age: when did the last backup finish, and would a day of studies between then and now be
  acceptable to lose?
