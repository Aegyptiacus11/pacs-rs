# CI and releases

Every push to `main` and every pull request runs `.github/workflows/ci.yml`: actionlint over the
workflows, the web interface, `cargo fmt`, clippy, and the tests on Linux (with Postgres, S3, the
reference server and the DIMSE interoperability tests) and on Windows. Each job runs only when
the change touches what it checks: `scripts/ci-changes.sh` reads the changed paths, so a change
to the documentation alone builds nothing, one to `ui/` builds the web interface only, and
anything else, a path the script does not know included, runs the Rust jobs.

This documentation is the mdBook in `docs/` (`book.toml`). `.github/workflows/docs.yml` checks its
links (`scripts/check-doc-links.py`) and builds it on every pull request that touches it, and
publishes it to GitHub Pages on a push to `main`. To read it while writing:

```bash
cargo install mdbook --version 0.5.4 --locked
mdbook serve --open
```

A release is a tag `v<version>` matching the workspace version in `Cargo.toml`:

```bash
git tag -a v0.5.2 -m v0.5.2 && git push origin v0.5.2
```

`.github/workflows/release.yml` first runs every job of `ci.yml` on the tagged commit, and stops
there if one fails. Then it publishes, once every build has passed:

- `pacs-rs-<version>-linux-amd64.tar.gz` and `-linux-arm64.tar.gz`: the static
  `pacs-server` binary, no dependencies;
- `pacs-rs-<version>-windows-amd64.zip`: `pacs-server.exe`, needs no Visual C++ runtime;
- the image `ghcr.io/aegyptiacus11/pacs-rs:<version>` for linux/amd64 and linux/arm64, and
  `:latest` for the highest release (a pre-release such as `v1.0.0-rc1` never gets it);
- a GitHub release with those files and `SHA256SUMS`.

Started by hand (Actions, *release*, *Run workflow*), the same workflow is a dry run: it tests,
builds and smoke-tests everything at the workspace version and publishes nothing.
