# CI and releases

Every push to `main` and every pull request runs `.github/workflows/ci.yml`: actionlint over the
workflows, the web interface, `cargo fmt`, clippy, and the tests on Linux (with Postgres, S3, the
reference server and the DIMSE interoperability tests) and on Windows. Each job runs only when
the change touches what it checks: `scripts/ci-changes.sh` reads the changed paths, so a change
to the documentation alone builds nothing, one to `ui/` builds the web interface only, and
anything else, a path the script does not know included, runs the Rust jobs.

This documentation is the mdBook in `docs/` (`book.toml`), served under `/docs/` of the site;
the homepage in front of it is `site/`, plain HTML and CSS. `scripts/build-site.py` builds the
book and puts the two together in `target/site`, with a redirect at each address a page of the
book had when the book was the whole site, and checks the homepage's links.
`.github/workflows/docs.yml` checks the documentation's links (`scripts/check-doc-links.py`),
builds the site and runs [the pages' examples](building.md#the-documentations-examples) on every
pull request that touches them, and publishes the site to GitHub Pages on a push to `main`. To
read the book while writing, and to see the whole site:

```bash
cargo install mdbook --version 0.5.4 --locked
mdbook serve --open
python3 scripts/build-site.py && python3 -m http.server -d target/site
```

Two pages are written by the code and held to it by a test: the [REST API
reference](api-reference.md) and the [Conformance Statement](conformance.md), which names the
version. A version change writes both again:

```bash
cargo run -p pacs-server --example api_reference
cargo run -p pacs-dimse --example conformance
```

A release is a tag `v<version>` matching the workspace version in `Cargo.toml`:

```bash
git tag -a v0.16.3 -m v0.16.3 && git push origin v0.16.3
```

`.github/workflows/release.yml` first runs every job of `ci.yml` on the tagged commit, and stops
there if one fails. Then it publishes, once every build has passed:

- `pacs-rs-<version>-linux-amd64.tar.gz` and `-linux-arm64.tar.gz`: the static
  `pacs-server` binary, no dependencies, and the [command-line tools](tools.md) beside it;
- `pacs-rs-<version>-windows-amd64.zip`: `pacs-server.exe` and the tools, needing no Visual
  C++ runtime;
- the image `ghcr.io/aegyptiacus11/pacs-rs:<version>` for linux/amd64 and linux/arm64, and
  `:latest` for the highest release (a pre-release such as `v1.0.0-rc1` never gets it);
- a GitHub release with those files and `SHA256SUMS`.

Once the image is published, the index it writes is kept for the upgrade tests: run
`scripts/upgrade-fixtures.sh <version>`, add the version to `RELEASES` in
`crates/pacs-index/tests/upgrades.rs`, and commit both. Every later release must open it.

Started by hand (Actions, *release*, *Run workflow*), the same workflow is a dry run: it tests,
builds and smoke-tests everything at the workspace version and publishes nothing.
