#!/usr/bin/env bash
# The backup and restore of docs/backup.md, run as written against real servers: a running
# pacs-server is backed up, the copy restored into a fresh place, a second server started on
# it, and what it serves checked. Between the index backup and the file copy, one instance
# arrives and another is deleted, as on a server in use; the restore's consistency repair
# must settle both.
#
#   scripts/backup-restore.sh sqlite      # SQLite index, files on disk
#   scripts/backup-restore.sh postgres    # Postgres index, files in S3
#
# Needs target/debug/pacs-server (cargo build -p pacs-server), curl, python3, docker.
# postgres: PACS_TEST_PG_CONTAINER (a running Postgres container, user
# postgres, password test, published as PACS_TEST_PG_URL) and PACS_TEST_S3_ENDPOINT (an S3
# server such as SeaweedFS, any access key); rclone runs from its image.
set -euo pipefail

cd "$(dirname "$0")/.."
kind=${1:?sqlite or postgres}
server=$PWD/target/debug/pacs-server
[ -x "$server" ] || { echo "build it first: cargo build -p pacs-server" >&2; exit 1; }
work=$(mktemp -d)
pids=()
cleanup() {
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  rm -rf "$work"
}
trap cleanup EXIT
export PACS_COMPRESSION=off PACS_TRASH_DAYS=30
sample=(docs/sample/00001.dcm docs/sample/00002.dcm docs/sample/00003.dcm)

# A server on $1 with the given environment, waited for.
start() {
  local port=$1; shift
  env "$@" PACS_LISTEN=127.0.0.1:$port "$server" >"$work/server-$port.log" 2>&1 &
  pids+=($!)
  for _ in $(seq 1 60); do curl -fs -o /dev/null "http://127.0.0.1:$port/healthz" && return 0; sleep 0.5; done
  cat "$work/server-$port.log" >&2
  echo "the server on $port did not start" >&2
  return 1
}
upload() { curl -fsS -o /dev/null -X POST --data-binary "@$2" -H 'content-type: application/dicom' "http://127.0.0.1:$1/instances"; }
instances() {
  curl -fsS "http://127.0.0.1:$1/studies" | python3 -c '
import sys, json, urllib.request
studies = json.load(sys.stdin)
port = sys.argv[1]
print(sum(len(json.load(urllib.request.urlopen(f"http://127.0.0.1:{port}/studies/{s}/instances"))) for s in studies))' "$1"
}
one_instance() {
  curl -fsS "http://127.0.0.1:$1/studies" | python3 -c '
import sys, json, urllib.request
port = sys.argv[1]
study = json.load(sys.stdin)[0]
print(json.load(urllib.request.urlopen(f"http://127.0.0.1:{port}/studies/{study}/instances"))[0]["ID"])' "$1"
}

case $kind in
sqlite)
  data=$work/data backup=$work/backup restore=$work/restore
  mkdir -p "$data" "$backup" "$restore"
  live=(PACS_INDEX=$data/index.sqlite PACS_STORAGE=file:$data/files)
  start 18101 "${live[@]}"
  upload 18101 "${sample[0]}"; upload 18101 "${sample[1]}"
  # --- docs/backup.md, With SQLite: a running server, index first, then the files
  # (the page's Python form of SQLite's online backup: no sqlite3 needed)
  python3 -c "
import sqlite3
src = sqlite3.connect('file:$data/index.sqlite?mode=ro', uri=True)
dst = sqlite3.connect('$backup/index.sqlite')
src.backup(dst)"
  # meanwhile: one instance arrives, one is deleted
  upload 18101 "${sample[2]}"
  curl -fsS -o /dev/null -X DELETE "http://127.0.0.1:18101/instances/$(one_instance 18101)"
  cp -a "$data/files" "$backup/files"
  # --- docs/backup.md, Restoring
  cp -a "$backup/index.sqlite" "$restore/index.sqlite"
  cp -a "$backup/files" "$restore/files"
  restored=(PACS_INDEX=$restore/index.sqlite PACS_STORAGE=file:$restore/files)
  ;;
postgres)
  : "${PACS_TEST_PG_CONTAINER:?}" "${PACS_TEST_PG_URL:?}" "${PACS_TEST_S3_ENDPOINT:?}"
  export PACS_S3_ENDPOINT=$PACS_TEST_S3_ENDPOINT PACS_S3_ACCESS_KEY_ID=test PACS_S3_SECRET_ACCESS_KEY=test
  suffix=$$
  db=pacs_live_$suffix restored_db=pacs_restored_$suffix
  bucket=pacs-live-$suffix restored_bucket=pacs-restored-$suffix
  psql() { docker exec -i "$PACS_TEST_PG_CONTAINER" psql -U postgres -q "$@"; }
  psql -c "CREATE DATABASE $db"
  curl -fsS -o /dev/null -X PUT "$PACS_TEST_S3_ENDPOINT/$bucket"
  curl -fsS -o /dev/null -X PUT "$PACS_TEST_S3_ENDPOINT/$restored_bucket"
  start 18101 PACS_INDEX="$PACS_TEST_PG_URL/$db" PACS_STORAGE="s3://$bucket"
  upload 18101 "${sample[0]}"; upload 18101 "${sample[1]}"
  # --- docs/backup.md, With Postgres and With S3: the index first, then the bucket
  docker exec "$PACS_TEST_PG_CONTAINER" pg_dump -U postgres -d "$db" | gzip >"$work/pacs.sql.gz"
  upload 18101 "${sample[2]}"
  curl -fsS -o /dev/null -X DELETE "http://127.0.0.1:18101/instances/$(one_instance 18101)"
  rclone() {
    docker run --rm --network host \
      -e RCLONE_CONFIG_S3_TYPE=s3 -e RCLONE_CONFIG_S3_PROVIDER=Other \
      -e RCLONE_CONFIG_S3_ENDPOINT="$PACS_TEST_S3_ENDPOINT" \
      -e RCLONE_CONFIG_S3_ACCESS_KEY_ID=test -e RCLONE_CONFIG_S3_SECRET_ACCESS_KEY=test \
      rclone/rclone:1.70.3 "$@"
  }
  rclone sync "s3:$bucket" "s3:$restored_bucket"
  # --- docs/backup.md, Restoring: into a new database, and the copy of the bucket
  psql -c "CREATE DATABASE $restored_db"
  gunzip -c "$work/pacs.sql.gz" | psql -d "$restored_db"
  restored=(PACS_INDEX="$PACS_TEST_PG_URL/$restored_db" PACS_STORAGE="s3://$restored_bucket")
  ;;
*) echo "sqlite or postgres" >&2; exit 1 ;;
esac

# --- docs/backup.md, Restoring: the store and the index made to agree, then checked
set +e
env "${restored[@]}" "$server" --consistency --grace 0 >"$work/before.txt"
before=$?
set -e
cat "$work/before.txt"
[ "$before" = 1 ] || { echo "the copy should differ from its index: one late file, one moved to the trash" >&2; exit 1; }
grep -q '^orphan: ' "$work/before.txt"
grep -q '^lost: ' "$work/before.txt"
env "${restored[@]}" "$server" --consistency --grace 0 --repair
env "${restored[@]}" "$server" --consistency --grace 0
env "${restored[@]}" "$server" --check-files

start 18102 "${restored[@]}"
# the late instance is in; the one deleted after the index backup went with the repair
test "$(instances 18102)" = 2
curl -fsS -o "$work/one.dcm" "http://127.0.0.1:18102/instances/$(one_instance 18102)/file"
python3 -c 'import sys; d=open(sys.argv[1],"rb").read(); assert d[128:132]==b"DICM", "not DICOM"' "$work/one.dcm"
echo "backup and restore ($kind): ok"
