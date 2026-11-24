# Deploying

From nothing to a server a department can rely on: the index on Postgres, the files in S3,
and how much of each to plan for. The rest of a real installation has its own pages:
[behind a reverse proxy](reverse-proxy.md), [backing up and upgrading](backup.md),
[the login](login.md), [the audit trail](audit.md), and [settings in a file](settings.md#in-a-file).

SQLite and a directory, the defaults, are right for one server with up to a few hundred
thousand images and a disk you back up. Postgres and S3 are for more than that, for
[several servers](running.md#several-servers) on one archive, or for storage you do not
manage yourself.

## Postgres from nothing

Any supported Postgres (CI tests 18). One database, and a role that owns it: pacs-rs makes
and changes its own tables as it starts, and needs nothing beyond its database.

```sql
CREATE ROLE pacs LOGIN PASSWORD '…';
CREATE DATABASE pacs OWNER pacs;
```

```bash
PACS_INDEX=postgres://pacs@db.internal:5432/pacs
PACS_INDEX_PASSWORD_FILE=/run/secrets/pacs-db   # the password, never in the URL of a setting file
PACS_INDEX_TLS=verify-full                      # and PACS_INDEX_CA_FILE when a private authority signs it
```

- **TLS**: [TLS to Postgres](running.md#tls-to-postgres) says what each mode checks. Across a
  network, `verify-full`.
- **No just-in-time compilation**: pacs-rs turns `jit` off on its own connections, since its
  queries are short and compiling one costs more than running it. Nothing to set on the server.
- **Backups**: `pg_dump` while the server runs, then the files, in that order
  ([backing up](backup.md#with-postgres)).
- **Connections**: each server keeps a small pool. Two servers on one database need nothing
  more ([several servers](running.md#several-servers)).

## S3 from nothing

A bucket of its own, and keys that reach that bucket only. pacs-rs reads, writes, lists,
copies (to move a deleted study to the [trash](trash.md)) and deletes objects in it, and does
nothing else with the account.

On AWS, a user or a role with this policy (`pacs-archive` being the bucket):

```json
{
  "Version": "2012-10-17",
  "Statement": [
    { "Effect": "Allow", "Action": ["s3:ListBucket"], "Resource": "arn:aws:s3:::pacs-archive" },
    {
      "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"],
      "Resource": "arn:aws:s3:::pacs-archive/*"
    }
  ]
}
```

```bash
PACS_STORAGE=s3://pacs-archive          # or s3://pacs-archive/a/prefix
PACS_S3_REGION=eu-central-1
PACS_S3_ACCESS_KEY_ID_FILE=/run/secrets/s3-key-id
PACS_S3_SECRET_ACCESS_KEY_FILE=/run/secrets/s3-secret
```

Another S3-compatible store (MinIO, SeaweedFS, Ceph, a cloud other than AWS) takes the same
settings and `PACS_S3_ENDPOINT=https://s3.example.org`; pacs-rs then asks for objects by
path (`https://s3.example.org/pacs-archive/…`), which these stores answer. CI runs every test
of the store against SeaweedFS.

- **Versioning** on the bucket keeps what pacs-rs overwrites or deletes for as long as you
  say: one more net under the [trash](trash.md), worth turning on.
- **Encryption at rest** is the store's: pacs-rs reads and writes the objects as they are.
- A quarantine (`PACS_QUARANTINE`) goes in another bucket, or another prefix that is not
  inside `PACS_STORAGE`.

## Sizing

Measured on 22 November 2026 with the 0.13.0 image on a laptop (Ryzen 7 PRO 7840U), 500 CT
images of 128 × 128 sent four at a time, compression off; the index does not depend on the
size of the pixels:

| | SQLite | Postgres |
|---|---|---|
| Memory of an idle server | 29 MB | 25 MB |
| Memory while receiving | 74 MB at the most | 61 MB at the most |
| The index, for each image | 17 KB | 6 KB |
| 500 images received | in 0.9 s | in 1.2 s |

**Storage.** Each image is kept as received, uncompressed pixel data compressed with lossless
JPEG-LS: on 172 slices of a public abdominal CT (512 × 512, 16-bit) it took 90.4 MB to
26.8 MB ([DICOMweb](dicomweb.md)). A CT of 300 to 1,000 slices of 512 × 512 is 150 to 500 MB
uncompressed, so about 45 to 150 MB stored. Add the [trash](trash.md): what was deleted in the
last `PACS_TRASH_DAYS` days (30).

**The index** grows with the number of images, not their size: about 6 KB each on Postgres,
17 KB on SQLite. A million images is about 6 GB of Postgres, before its own indexes and
free space.

**Memory** is what the requests being served need: the server reads a file whole to answer
for it, and the most it reads at once is bounded by `PACS_MAX_READ_BYTES` (512 MiB), beside
the files themselves. So a server needs its idle size, the files being read, and up to that
bound: 1 GB leaves room for it. That is an estimate from the bound, not a measurement of a
department's load.

**CPU** goes to compressing what arrives (a CT slice takes about 2.5 ms on one core with
codecsls, as its README measures) and to decoding what viewers ask for in another form. How
many cores a department needs was not measured: watch the [metrics](running.md#metrics) and
the [readiness check](running.md) on your own studies before relying on any figure here.
