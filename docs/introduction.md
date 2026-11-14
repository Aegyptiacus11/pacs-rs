# pacs-rs

An open-source PACS written in Rust, built on the [dicom-rs](https://github.com/Enet4/dicom-rs)
crates. Its index is SQLite or Postgres; its files are on disk or in S3-compatible storage.

Status: early development. `pacs-server` serves DICOMweb and a REST API over HTTP, with files on
disk or in S3 and a SQLite or Postgres index, a web interface with a viewer, and speaks DIMSE:
C-ECHO, C-STORE into the archive, and C-FIND and C-MOVE to search another archive's studies and
pull them in, de-identified on the way in when a project asks for it.

## Where to start

| To | Read |
|---|---|
| start a server and set it up | [Running](running.md) |
| ask for a login, and manage users and roles | [Login](login.md) |
| browse and view studies | [Web interface](web-interface.md) |
| connect a viewer or a pipeline over HTTP | [DICOMweb](dicomweb.md), [REST API](rest-api.md) |
| connect a hospital's PACS or a workstation | [DIMSE](dimse.md) |
| de-identify what a project pulls in | [De-identification](de-identification.md) |
| see what is not built yet | [Roadmap](roadmap.md) |
| build, test or release it | [Building and testing](building.md), [CI and releases](releases.md) |

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

`ui/` is the [web interface](web-interface.md), a Vite project built into the binary.

Each crate's items are documented in the code; `cargo doc --no-deps --workspace --open` builds
that reference.

## License

MIT or Apache-2.0, at your option; see the
[repository](https://github.com/Aegyptiacus11/pacs-rs).
