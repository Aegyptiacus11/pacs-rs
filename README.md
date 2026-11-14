# pacs-rs

An open-source PACS written in Rust, built on the [dicom-rs](https://github.com/Enet4/dicom-rs)
crates. Its index is SQLite or Postgres; its files are on disk or in S3-compatible storage.

Status: early development. `pacs-server` serves DICOMweb and a REST API over HTTP, with files on
disk or in S3 and a SQLite or Postgres index, a web interface with a viewer, and speaks DIMSE:
C-ECHO, C-STORE into the archive, and C-FIND and C-MOVE to search another archive's studies and
pull them in, de-identified on the way in when a project asks for it.

**Documentation: <https://aegyptiacus11.github.io/pacs-rs/>** (its pages are in [docs/](docs)).

## Try it

With Docker Compose, from the published image (no build):

```bash
curl -fsSLO https://raw.githubusercontent.com/Aegyptiacus11/pacs-rs/main/compose.yaml
docker compose up -d --wait
```

The web interface is then at `http://127.0.0.1:8080/ui/` and DICOMweb at
`http://127.0.0.1:8080/dicom-web`. From source:

```bash
(cd ui && npm ci && npm run build)
cargo run --release -p pacs-server
```

**The login is off by default:** anyone who can reach the server can then delete. The compose
file publishes the port on 127.0.0.1 only; turn the [login](docs/login.md) on (`PACS_AUTH`) or put
an authenticating proxy in front before opening it to a network.

## Documentation

| | |
|---|---|
| [Running](docs/running.md) | Docker, the image, every `PACS_*` setting |
| [Login](docs/login.md) | `PACS_AUTH`: signing in, roles, users |
| [Web interface](docs/web-interface.md) | the worklist, patients, the viewer, hanging protocols, measurements |
| [DICOMweb](docs/dicomweb.md) | QIDO-RS, WADO-RS, STOW-RS; compression, transcoding, caching |
| [REST API](docs/rest-api.md) | resources by public ID, uploads, deletion, find, archives, remote AEs |
| [DIMSE](docs/dimse.md) | C-ECHO, C-STORE, C-FIND and C-MOVE; statuses, AE titles |
| [De-identification](docs/de-identification.md) | projects, PS3.15 profiles and options, quarantine |
| [Roadmap](docs/roadmap.md) | what is not built yet |
| [Hospital interfaces: the plan](docs/interop.md) | the research PACS beside a clinical one, and its build order |
| [Building and testing](docs/building.md) | the toolchain, the web interface, Postgres, S3, DIMSE and reference tests |
| [CI and releases](docs/releases.md) | CI, the docs site, tags, archives and the image |
| [Benchmarks](docs/benchmarks.md) | measurements and their limits |

## Crates

| Crate | What |
|---|---|
| `pacs-core` | parse a DICOM file, its UIDs, per-level attributes, DICOM JSON, frames; lossless codecs and transcoding |
| `pacs-store` | the files, as received: local disk, S3-compatible storage or memory |
| `pacs-index` | the index, SQLite or Postgres: hierarchy, counts, attributes, metadata, storage keys |
| `pacs-archive` | ingest (compress, store, then index, undone if the index refuses) and deletion (index then files) |
| `pacs-dicomweb` | QIDO-RS, WADO-RS, STOW-RS over axum |
| `pacs-rest` | the REST API: resources by public ID, uploads, deletion, find, archives |
| `pacs-dimse` | DIMSE on dicom-ul: C-ECHO SCP and SCU, C-STORE SCP into the archive, C-FIND and C-MOVE SCU |
| `pacs-deid` | de-identification by PS3.15 Annex E, configured per project; no network or storage, usable on its own |
| `pacs-server` | the binary: settings, `/healthz`, the REST API at `/`, DICOMweb under `/dicom-web`, the web interface at `/ui/` |

`ui/` is the web interface, a Vite project built into the binary.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
