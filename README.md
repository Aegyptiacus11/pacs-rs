# pacs-rs

An open-source PACS written in Rust, built on the [dicom-rs](https://github.com/Enet4/dicom-rs)
crates.

Status: early development. `pacs-server` serves DICOMweb over HTTP, with files on disk or in
S3 and a SQLite index.

## Crates

| Crate | What |
|---|---|
| `pacs-core` | parse a DICOM file, its UIDs, per-level attributes, DICOM JSON, frames |
| `pacs-store` | the files, as received: local disk, S3-compatible storage or memory |
| `pacs-index` | the SQLite index: hierarchy, counts, attributes, metadata, storage keys |
| `pacs-archive` | ingest: store then index, undone if the index refuses |
| `pacs-dicomweb` | QIDO-RS, WADO-RS, STOW-RS over axum |
| `pacs-rest` | the REST API: resources by public ID, uploads, find, study archives |
| `pacs-server` | the binary: settings, `/healthz`, the REST API at `/`, DICOMweb under `/dicom-web` |

## DICOMweb

| Request | |
|---|---|
| `GET /studies`, `/studies/{study}/series`, `.../series/{series}/instances` | QIDO-RS; counts, Modalities in Study and Retrieve URL always included |
| `GET /studies/{study}`, `.../series/{series}`, `.../instances/{instance}` | WADO-RS retrieve, `multipart/related; type="application/dicom"`, as stored |
| `GET .../metadata` (study, series, instance) | DICOM JSON without pixel data |
| `GET .../instances/{instance}/frames/{list}` | frames in the stored transfer syntax: `application/octet-stream` or the compressed type of PS3.18 Table 8.7.3-5 |
| `POST /studies`, `POST /studies/{study}` | STOW-RS, `application/dicom` parts |

Not yet: wildcard, range, list and fuzzy matching (answered with 400, not ignored);
`includefield` for attributes beyond the stored levels; transcoding (406 when the client wants
a transfer syntax other than the stored one); rendered media; JSON/XML + bulk data STOW;
BulkDataURI in metadata.

DICOM JSON numbers: IS and DS values are written as JSON numbers, padding trimmed (PS3.18 Table
F.2.3-1 allows numbers or strings).

## REST API

At the server's root, compatible with the reference server's REST API (see "Comparing with a
reference server"), so its clients work unchanged. Resources have public IDs: the SHA-1 of their
DICOM identifiers (`pacs_core::public_ids`), the same as the reference server gives them.

| Request | |
|---|---|
| `POST /instances` | one DICOM file as the body; `ID`, `ParentPatient`, `ParentStudy`, `ParentSeries`, `Path`, `Status` (`Success` or `AlreadyStored`) |
| `GET /studies[?expand][&limit=&since=]` | study IDs, or study objects |
| `GET /studies/{id}`, `/series/{id}`, `/instances/{id}` | the resource, with its `MainDicomTags` |
| `GET /studies/{id}/series`, `/studies/{id}/instances`, `/series/{id}/instances` | children, expanded |
| `GET /studies/{id}/statistics` | counts and sizes |
| `GET /studies/{id}/archive` | the study's files in a ZIP, by patient, study and series |
| `GET /instances/{id}/file` | the DICOM file as stored |
| `POST /tools/find` | `Level` Study, Series or Instance, exact matching on the UIDs, PatientID, AccessionNumber, StudyDate |

An instance sent again is kept as first stored (`AlreadyStored`) unless `PACS_OVERWRITE_INSTANCES`
is set. Lists of a study's series and a series' instances come in Series and Instance Number
order.

Not yet: patients, labels, metadata, modifications and deletion, wildcard and range searches in
`/tools/find`.

DIMSE services (C-ECHO, C-STORE, C-FIND, C-MOVE over dicom-ul) come after.

## Running

```bash
cargo run --release -p pacs-server -- --help
cargo run --release -p pacs-server            # files in data/files, index in data/index.sqlite
```

Every flag has a `PACS_*` environment variable:

| Variable | Default | |
|---|---|---|
| `PACS_LISTEN` | `127.0.0.1:8080` | address to listen on |
| `PACS_DICOMWEB_PATH` | `/dicom-web` | where the DICOMweb root is served |
| `PACS_BASE_URL` | from the request | public URL of the DICOMweb root, for Retrieve URLs |
| `PACS_INDEX` | `data/index.sqlite` | the SQLite index |
| `PACS_STORAGE` | `file:data/files` | `file:<directory>` or `s3://<bucket>[/<prefix>]` |
| `PACS_S3_ENDPOINT` | AWS | e.g. `http://homeserver:8333` for SeaweedFS |
| `PACS_S3_REGION` | `us-east-1` | |
| `PACS_S3_ACCESS_KEY_ID`, `PACS_S3_SECRET_ACCESS_KEY` | | environment only, or `..._FILE` naming a file |
| `PACS_OVERWRITE_INSTANCES` | `false` | replace an instance sent again, instead of keeping the first copy |
| `PACS_MAX_UPLOAD_BYTES` | 1 GiB | largest STOW-RS or upload body |
| `PACS_CORS_ORIGINS` | none | comma-separated origins, or `*` |
| `RUST_LOG` | `info` | log filter |

In Docker, `docker build -t pacs-server .`; the image listens on `0.0.0.0:8080` and keeps the
index and files in the `/data` volume unless `PACS_STORAGE` says otherwise.

A synthetic series to try it with:

```bash
cargo run -p pacs-core --features synthetic --example write_synthetic -- synthetic 1 3
curl -X POST --data-binary @synthetic/00001.dcm http://127.0.0.1:8080/instances
```

## Building

The toolchain is pinned in `rust-toolchain.toml`; rustup picks it up.

```bash
cargo test --workspace
```

The S3 tests run only when `PACS_TEST_S3_ENDPOINT` is set; against a throwaway SeaweedFS:

```bash
docker run -d --rm --name pacs-rs-test-s3 -p 127.0.0.1:18333:8333 chrislusf/seaweedfs:4.48 server -s3
curl -X PUT http://127.0.0.1:18333/pacs-test
PACS_TEST_S3_ENDPOINT=http://127.0.0.1:18333 cargo test --workspace
docker stop pacs-rs-test-s3
```

## Comparing with a reference server

`crates/pacs-server/tests/reference.rs` stores the same synthetic files in pacs-rs and in a
reference DICOMweb server, makes the same requests to both and compares the answers tag by tag,
frame by frame and file by file. The reference is Orthanc 26.6.1 with its DICOMweb plugin;
pacs-rs aims to be compatible with it. Deliberate differences are listed in the test, each with
its reason. The test runs only when `PACS_TEST_REFERENCE_URL` is set:

```bash
docker run -d --rm --name pacs-rs-reference -p 127.0.0.1:18042:8042 \
  -e DICOM_WEB_PLUGIN_ENABLED=true -e ORTHANC__AUTHENTICATION_ENABLED=false \
  orthancteam/orthanc:26.6.1
PACS_TEST_REFERENCE_URL=http://127.0.0.1:18042/dicom-web cargo test -p pacs-server --test reference
docker stop pacs-rs-reference
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
