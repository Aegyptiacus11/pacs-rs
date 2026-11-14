#!/usr/bin/env bash
# Tests of ci-changes.sh. Exits 1 at the first case that answers otherwise.
set -euo pipefail
script="$(dirname "$0")/ci-changes.sh"

# expect <rust> <ui> <what> <path>...
expect() {
  local rust=$1 ui=$2 what=$3 got
  shift 3
  got=$(printf '%s\n' "$@" | bash "$script" | tr '\n' ' ')
  if [ "$got" != "rust=$rust ui=$ui " ]; then
    echo "FAILED: $what: expected rust=$rust ui=$ui, got $got" >&2
    exit 1
  fi
}

expect false false "documentation only" docs/roadmap.md README.md book.toml LICENSE-MIT \
  scripts/check-doc-links.py scripts/test_check_doc_links.py .github/workflows/docs.yml
expect false false "the release workflow" .github/workflows/release.yml
expect false true "the web interface only" ui/src/App.tsx ui/package-lock.json ui/README.md
expect true false "a crate" crates/pacs-core/src/lib.rs
expect true false "the lock file" Cargo.lock
expect true false "a path nobody listed" something/new.txt
expect true false "the CMake toolchain file" cmake/msvc-static-crt.cmake
expect true true "a crate and the web interface" docs/dimse.md crates/pacs-dimse/src/lib.rs ui/index.html
expect true true "the workflow itself" .github/workflows/ci.yml
expect true true "this filter" README.md scripts/ci-changes.sh
expect true true "its tests" scripts/test-ci-changes.sh
expect true true "no path at all" ""

# a last line without a newline still counts
got=$(printf 'docs/a.md\ncrates/x.rs' | bash "$script" | tr '\n' ' ')
[ "$got" = "rust=true ui=false " ] || { echo "FAILED: no final newline: got $got" >&2; exit 1; }

got=$(bash "$script" --all < /dev/null | tr '\n' ' ')
[ "$got" = "rust=true ui=true " ] || { echo "FAILED: --all: got $got" >&2; exit 1; }

# a wrong argument is refused, with nothing on standard output
for arguments in "--everything" "--all extra"; do
  status=0
  # shellcheck disable=SC2086  # the words are separate arguments
  out=$(bash "$script" $arguments < /dev/null 2> /dev/null) || status=$?
  if [ "$status" -ne 2 ] || [ -n "$out" ]; then
    echo "FAILED: '$arguments': expected exit 2 and no output, got $status and '$out'" >&2
    exit 1
  fi
done

echo "ci-changes.sh: all cases pass"
