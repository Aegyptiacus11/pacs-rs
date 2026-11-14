#!/usr/bin/env bash
# Says which jobs of .github/workflows/ci.yml a change needs, from the paths it touches.
# Reads the changed paths on standard input, one a line, and prints `rust=true|false` and
# `ui=true|false`, the form of $GITHUB_OUTPUT. `--all` prints both true without reading.
#
# - the documentation and the release and docs workflows need neither (docs.yml checks the
#   docs; the `workflows` job lints every workflow);
# - ui/ needs the web interface job only: the Rust tests do not build it;
# - ci.yml and this script need both;
# - anything else needs the Rust jobs, and so does a path this script does not know.
# No path at all says nothing about the change, so both are true.
set -euo pipefail

if [ "$#" -gt 1 ] || { [ "$#" -eq 1 ] && [ "$1" != "--all" ]; }; then
  echo "usage: ci-changes.sh [--all] < changed paths" >&2
  exit 2
fi

rust=false
ui=false
paths=0
if [ "$#" -eq 1 ]; then
  rust=true
  ui=true
  paths=1
else
  while IFS= read -r path || [ -n "$path" ]; do
    [ -n "$path" ] || continue
    paths=$((paths + 1))
    case "$path" in
      .github/workflows/ci.yml | scripts/ci-changes.sh | scripts/test-ci-changes.sh)
        rust=true
        ui=true
        ;;
      docs/* | *.md | book.toml | LICENSE-* | scripts/check-doc-links.py | \
        scripts/test_check_doc_links.py | .github/workflows/docs.yml | \
        .github/workflows/release.yml) ;;
      ui/*) ui=true ;;
      *) rust=true ;;
    esac
  done
fi

if [ "$paths" -eq 0 ]; then
  rust=true
  ui=true
fi
echo "rust=$rust"
echo "ui=$ui"
