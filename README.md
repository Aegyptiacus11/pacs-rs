# pacs-rs

An open-source PACS written in Rust, built on the [dicom-rs](https://github.com/Enet4/dicom-rs)
crates. Its index is SQLite or Postgres; its files are on disk or in S3-compatible storage.

Status: early development. `pacs-server` serves DICOMweb and a REST API over HTTP, with files on
disk or in S3 and a SQLite or Postgres index, a web interface with a viewer, and speaks DIMSE:
C-ECHO, C-STORE into the archive, and C-FIND and C-MOVE to search another archive's studies and
pull them in, de-identified on the way in when a project asks for it.

**Site: <https://aegyptiacus11.github.io/pacs-rs/>**, and the documentation at
<https://aegyptiacus11.github.io/pacs-rs/docs/> (its pages are in [docs/](docs)).

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
| [Quickstart](docs/quickstart.md) | a server, a first study in it, on screen and over HTTP |
| [Two ways to use pacs-rs](docs/two-ways.md) | what to read for a research archive, and for a clinical one |
| [DICOM: files and studies](docs/dicom-files.md), [the network](docs/dicom-network.md) | what DICOM is, shown on the sample study, with the standard's sections |
| [REST API, task by task](docs/rest-guide.md) | sending, listing, fetching, searching and deleting, with the answers |
| [Running](docs/running.md) | Docker, the image, TLS to Postgres, limits on requests |
| [Behind a reverse proxy](docs/reverse-proxy.md) | what the proxy must do, and what pacs-rs must be told |
| [Backing up and upgrading](docs/backup.md) | the index and the files, in which order; restoring; what an upgrade changes |
| [Settings](docs/settings.md) | every `PACS_*` setting, by what it is about |
| [Login](docs/login.md) | `PACS_AUTH`: signing in, roles, users |
| [Audit trail](docs/audit.md) | `PACS_AUDIT_FILE`: who signed in, and who stored, read, searched, exported and deleted what, as DICOM audit messages |
| [Web interface](docs/web-interface.md) | the worklist, patients, the viewer, hanging protocols, measurements |
| [DICOMweb](docs/dicomweb.md) | QIDO-RS, WADO-RS, STOW-RS; compression, transcoding, caching |
| [REST API reference](docs/api-reference.md) | every request in full, written from the code; also served at `/openapi.json` |
| [REST API](docs/rest-api.md) | the reference: resources by public ID, uploads, deletion, find, archives, remote AEs |
| [DIMSE](docs/dimse.md) | C-ECHO, C-STORE, C-FIND and C-MOVE; statuses, AE titles |
| [De-identification](docs/de-identification.md) | projects, PS3.15 profiles and options, quarantine |
| [Roadmap](docs/roadmap.md) | what is not built yet, towards a research and a clinical PACS in one |
| [Hospital interfaces: the plan](docs/interop.md) | the research PACS beside a clinical one, the clinical archive after it, and the build order |
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
| `pacs-audit` | DICOM audit trail messages (PS3.15 A.5) as the syslog lines of IHE ATNA; no network or storage, usable on its own |
| `pacs-deid` | de-identification by PS3.15 Annex E, configured per project; no network or storage, usable on its own |
| `pacs-server` | the binary: settings, `/healthz`, the REST API at `/`, DICOMweb under `/dicom-web`, the web interface at `/ui/` |

`ui/` is the web interface, a Vite project built into the binary.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

The sample study in `docs/sample/` and the pictures that show it are not under these
licences: they are adapted from a public research dataset and shared under CC BY-NC-SA 4.0
([The sample study](docs/sample-study.md)).

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
