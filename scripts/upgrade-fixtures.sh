#!/usr/bin/env bash
# The indexes the released pacs-rs images wrote, for crates/pacs-index/tests/upgrades.rs:
# each release from 0.5.2 on (the first one published to ghcr.io) is run, once on SQLite and
# once on Postgres, is sent the sample study of docs/sample, and is stopped; its index is
# kept as crates/pacs-index/tests/upgrades/<version>.sqlite and <version>.postgres.sql
# (pg_dump with INSERT statements, which the test can run without psql).
#
#   scripts/upgrade-fixtures.sh 0.12.0 [0.13.0 ...]   # these releases only
#   scripts/upgrade-fixtures.sh                         # every release
#
# Needs docker, curl and python3. A copy that exists is kept: delete it to make it again.
set -euo pipefail

cd "$(dirname "$0")/.."
out=crates/pacs-index/tests/upgrades
mkdir -p "$out"
releases=("$@")
if [ ${#releases[@]} -eq 0 ]; then
  releases=(0.5.2 0.6.0 0.7.0 0.8.0 0.8.1 0.9.0 0.10.0 0.11.0 0.12.0 0.13.0 0.14.0)
fi
postgres_image=postgres:18.6-alpine
work=$(mktemp -d)
net=pacs-upgrades-$$
cleanup() {
  docker rm -f pacs-upgrades-server pacs-upgrades-pg >/dev/null 2>&1 || true
  docker network rm "$net" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT
docker network create "$net" >/dev/null

ready() {
  for _ in $(seq 1 60); do "$@" >/dev/null 2>&1 && return 0; sleep 1; done
  echo "not ready: $*" >&2
  return 1
}

# Send the sample study to the server on :18099, then stop it.
fill_and_stop() {
  ready curl -fsS http://127.0.0.1:18099/healthz
  for f in docs/sample/*.dcm; do
    curl -fsS -o /dev/null -X POST --data-binary "@$f" -H 'content-type: application/dicom' \
      http://127.0.0.1:18099/instances
  done
  # every release has these two: the study, and its three instances
  study=$(curl -fsS http://127.0.0.1:18099/studies | python3 -c 'import sys,json;s=json.load(sys.stdin);assert len(s)==1,s;print(s[0])')
  test "$(curl -fsS "http://127.0.0.1:18099/studies/$study/instances" | python3 -c 'import sys,json;print(len(json.load(sys.stdin)))')" = 3
  docker stop -t 20 pacs-upgrades-server >/dev/null
  docker rm pacs-upgrades-server >/dev/null
}

for version in "${releases[@]}"; do
  image=ghcr.io/aegyptiacus11/pacs-rs:$version

  if [ ! -e "$out/$version.sqlite" ]; then
    data=$work/sqlite-$version
    mkdir -p "$data"
    docker run -d --name pacs-upgrades-server --user "$(id -u):$(id -g)" \
      -p 127.0.0.1:18099:8080 -v "$data:/data" "$image" >/dev/null
    fill_and_stop
    # one file, the write-ahead log folded in
    python3 -c 'import sqlite3,sys; sqlite3.connect(sys.argv[1]).execute("VACUUM INTO ?", (sys.argv[2],))' \
      "$data/index.sqlite" "$out/$version.sqlite"
    echo "$out/$version.sqlite"
  fi

  if [ ! -e "$out/$version.postgres.sql" ]; then
    docker run -d --name pacs-upgrades-pg --network "$net" -e POSTGRES_PASSWORD=test \
      "$postgres_image" >/dev/null
    ready docker exec pacs-upgrades-pg pg_isready -U postgres -q
    docker exec pacs-upgrades-pg createdb -U postgres pacs
    data=$work/postgres-$version
    mkdir -p "$data"
    docker run -d --name pacs-upgrades-server --user "$(id -u):$(id -g)" --network "$net" \
      -p 127.0.0.1:18099:8080 -v "$data:/data" \
      -e PACS_INDEX=postgres://postgres:test@pacs-upgrades-pg/pacs "$image" >/dev/null
    fill_and_stop
    docker exec pacs-upgrades-pg pg_dump -U postgres --inserts --no-owner --no-privileges \
      --no-comments pacs | grep -v '^\\' >"$out/$version.postgres.sql"  # \restrict: psql's, a new key each dump
    docker rm -f pacs-upgrades-pg >/dev/null
    echo "$out/$version.postgres.sql"
  fi
done
