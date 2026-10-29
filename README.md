# pacs-rs

An open-source PACS written in Rust, built on the [dicom-rs](https://github.com/Enet4/dicom-rs)
crates. Its index is SQLite or Postgres; its files are on disk or in S3-compatible storage.

Status: early development. `pacs-server` serves DICOMweb over HTTP, with files on disk or in
S3 and a SQLite index.

## Crates

| Crate | What |
|---|---|
| `pacs-core` | parse a DICOM file, its UIDs, per-level attributes, DICOM JSON, frames; lossless codecs and transcoding |
| `pacs-store` | the files, as received: local disk, S3-compatible storage or memory |
| `pacs-index` | the index, SQLite or Postgres: hierarchy, counts, attributes, metadata, storage keys |
| `pacs-archive` | ingest (compress, store, then index, undone if the index refuses) and deletion (index then files) |
| `pacs-dicomweb` | QIDO-RS, WADO-RS, STOW-RS over axum |
| `pacs-rest` | the REST API: resources by public ID, uploads, deletion, find, archives |
| `pacs-server` | the binary: settings, `/healthz`, the REST API at `/`, DICOMweb under `/dicom-web` |

## DICOMweb

| Request | |
|---|---|
| `GET /studies`, `/studies/{study}/series`, `.../series/{series}/instances` | QIDO-RS; counts, Modalities in Study and Retrieve URL always included |
| `GET /studies/{study}`, `.../series/{series}`, `.../instances/{instance}` | WADO-RS retrieve, `multipart/related; type="application/dicom"`, as stored or transcoded (below) |
| `GET .../metadata` (study, series, instance) | DICOM JSON without pixel data |
| `GET .../instances/{instance}/frames/{list}` | frames as stored or transcoded: `application/octet-stream` or the compressed type of PS3.18 Table 8.7.3-5 |
| `POST /studies`, `POST /studies/{study}` | STOW-RS, `application/dicom` parts |

Not yet: wildcard, range, list and fuzzy matching (answered with 400, not ignored);
`includefield` for attributes beyond the stored levels; rendered media; JSON/XML + bulk data
STOW; BulkDataURI in metadata.

### Compression and transcoding

Uncompressed pixel data is stored as **JPEG-LS lossless** (`1.2.840.10008.1.2.4.80`, CharLS):
each frame is decoded back and compared with what arrived before the file is kept, and an
instance that does not come back exact, or that the codecs do not handle (colour, more than 16
bits), is stored as received. So is anything that arrives compressed, **JPEG XL** included.
`PACS_COMPRESSION=off` stores everything as received.

A retrieve goes out as stored whenever the client accepts it (a viewer asking for
`transfer-syntax=*` gets the stored JPEG-LS untouched). Otherwise instances and frames are
transcoded to the first of these the client asks for, and anything else is 406:

| Transfer syntax | Frames | Decoded by | Encoded by |
|---|---|---|---|
| Explicit VR Little Endian `1.2.840.10008.1.2.1` | `application/octet-stream` | - | - |
| JPEG-LS lossless `1.2.840.10008.1.2.4.80` (`.81` read) | `image/jls` | CharLS | CharLS |
| JPEG XL lossless `1.2.840.10008.1.2.4.110` (`.112` read) | `image/jxl` | jxl-rs | zune-jpegxl |

Grayscale, 8 or 16 bits allocated; signed pixels are coded as their raw bit pattern, as DCMTK and
GDCM do.

JPEG XL is served and accepted, not chosen for storage. On 172 slices of a public CT (3D-IRCADb
patient 1.2, signed 16-bit, one core) JPEG-LS took the pixels to 1/3.38 at about 5 ms a slice;
zune-jpegxl, the pure-Rust encoder, reached 1/2.82, and libjxl at effort 7 1/3.37 at about
100 ms a slice. Through the server the study went from 90.4 MB to 26.8 MB, ingested in 3.3 s,
and every frame came back exact as uncompressed, JPEG-LS and JPEG XL (checked with CharLS and
libjxl, not only with the codecs above); jxl-rs decoded libjxl's own effort-7 output of the same
slices exactly.

DICOM JSON numbers: IS and DS values are written as JSON numbers, padding trimmed (PS3.18 Table
F.2.3-1 allows numbers or strings).

## REST API

At the server's root, compatible with the reference server's REST API (see "Comparing with a
reference server"), so its clients work unchanged. Resources have public IDs: the SHA-1 of their
DICOM identifiers (`pacs_core::public_ids`), the same as the reference server gives them.

| Request | |
|---|---|
| `POST /instances` | one DICOM file as the body; `ID`, `ParentPatient`, `ParentStudy`, `ParentSeries`, `Path`, `Status` (`Success` or `AlreadyStored`) |
| `GET /patients[?expand][&limit=&since=]`, `/studies[...]` | patient or study IDs, or the objects |
| `GET /patients/{id}`, `/studies/{id}`, `/series/{id}`, `/instances/{id}` | the resource, with its `MainDicomTags` |
| `GET /patients/{id}/studies`, `/patients/{id}/series`, `/patients/{id}/instances`, `/studies/{id}/series`, `/studies/{id}/instances`, `/series/{id}/instances` | descendants, expanded |
| `GET /studies/{id}/patient`, `/series/{id}/study`, `/series/{id}/patient`, `/instances/{id}/series`, `/instances/{id}/study`, `/instances/{id}/patient` | the ancestor |
| `GET /patients/{id}/statistics`, `/studies/{id}/statistics` | counts and sizes |
| `GET /patients/{id}/archive`, `/studies/{id}/archive` | the files in a ZIP, by patient, study and series |
| `GET /instances/{id}/file` | the DICOM file as stored |
| `DELETE /patients/{id}`, `/studies/{id}`, `/series/{id}`, `/instances/{id}` | the resource, everything under it and every ancestor left empty; `RemainingAncestor` names the closest one still there, or is `null` |
| `POST /tools/find` | `Level` Patient, Study, Series or Instance, exact matching on the UIDs, PatientID, AccessionNumber, StudyDate |

A patient is the studies that share its Patient ID. A deletion removes the index entries first,
then the files, so the index never names a missing file. A file that cannot be removed is
logged and left in the store, unindexed. Uploads wait while a deletion runs, so a file sent
again during a deletion is not lost. This holds within one server process: two pacs-rs
processes on one index do not coordinate yet.

**There is no login yet:** anyone who can reach the REST API can delete. Keep it on a private
network (see "Running").

An instance sent again is kept as first stored (`AlreadyStored`) unless `PACS_OVERWRITE_INSTANCES`
is set. Lists of a study's series and a series' instances come in Series and Instance Number
order.

Not yet: labels, metadata, modifications, wildcard and range searches in `/tools/find`.

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
| `PACS_INDEX` | `data/index.sqlite` | a SQLite file, or `postgres://user@host/database` |
| `PACS_INDEX_PASSWORD` | | the Postgres password; environment only, or `..._FILE` naming a file |
| `PACS_STORAGE` | `file:data/files` | `file:<directory>` or `s3://<bucket>[/<prefix>]` |
| `PACS_S3_ENDPOINT` | AWS | e.g. `http://homeserver:8333` for SeaweedFS |
| `PACS_S3_REGION` | `us-east-1` | |
| `PACS_S3_ACCESS_KEY_ID`, `PACS_S3_SECRET_ACCESS_KEY` | | environment only, or `..._FILE` naming a file |
| `PACS_OVERWRITE_INSTANCES` | `false` | replace an instance sent again, instead of keeping the first copy |
| `PACS_COMPRESSION` | `jpeg-ls` | how uncompressed pixel data is stored: `jpeg-ls` (lossless, checked) or `off` |
| `PACS_MAX_UPLOAD_BYTES` | 1 GiB | largest STOW-RS or upload body |
| `PACS_CORS_ORIGINS` | none | comma-separated origins, or `*` |
| `RUST_LOG` | `info` | log filter |

In Docker, `docker build -t pacs-server .`: a static binary on an empty image, about 20 MB, run as
an unprivileged user (uid 10001). It listens on `0.0.0.0:8080` and keeps the index and files in
the `/data` volume unless `PACS_STORAGE` says otherwise. With no shell or curl inside, health
checks call `/healthz` from outside.

A synthetic series to try it with:

```bash
cargo run -p pacs-core --features synthetic --example write_synthetic -- synthetic 1 3
curl -X POST --data-binary @synthetic/00001.dcm http://127.0.0.1:8080/instances
```

## Releases

Pushing a tag `v<version>` (the workspace version in `Cargo.toml`) releases to the homeserver
(`.github/workflows/image.yml`): it builds the Docker image, pushes it to the private registry
there as `pacs/pacs-server:<version>-<commit>` (and `latest`, when it is the highest release
version; pre-releases like `v1.0.0-rc1` never get it), pins that digest in homeserver-infra's
`stacks/pacs/compose.yml` (a commit to its `main`), calls the `pacs` stack's Portainer webhook,
and waits for `/healthz`. The job runs on a self-hosted runner on the homeserver
(homeserver-infra, `stacks/runners`). Repository secrets:

| Secret | What | Made with |
|---|---|---|
| `HARBOR_USERNAME`, `HARBOR_PASSWORD` | registry robot, push and pull in `pacs` | homeserver-infra `scripts/harbor-ci-robot.py` |
| `INFRA_TOKEN` | fine-grained token, Contents: read and write on homeserver-infra only | GitHub settings |
| `PORTAINER_PACS_WEBHOOK` | the `pacs` stack's webhook id | Portainer, the stack's page (homeserver-infra `stacks/pacs/README.md`) |

```bash
git tag v0.1.1 && git push origin v0.1.1
```

**Before this repository goes public**, remove the runner (homeserver-infra,
`stacks/runners/README.md`), revoke `INFRA_TOKEN`, delete the four secrets, and drop the
workflow or move it to GitHub's runners: the runner's build daemon is privileged, which is
root on the homeserver, and in a public repository a pull request can add a workflow that
runs on it.

## Building

The toolchain is pinned in `rust-toolchain.toml`; rustup picks it up. CharLS is C++, built by
the `charls` crate and linked statically, so building also needs `cmake` and a C++ compiler.

```bash
cargo test --workspace
```

The Postgres tests run only when `PACS_TEST_POSTGRES_URL` names a server; each test makes a
database of its own there. Every index test, the server tests and the reference comparison then
run on Postgres as well as SQLite:

```bash
docker run -d --rm --name pacs-rs-test-pg -p 127.0.0.1:15432:5432 -e POSTGRES_PASSWORD=test \
  postgres:18.6-alpine
PACS_TEST_POSTGRES_URL=postgres://postgres:test@127.0.0.1:15432 cargo test --workspace
docker stop pacs-rs-test-pg
```

Connections to Postgres are not encrypted yet (no TLS); keep the database on the same host or
private network.

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
frame by frame and file by file. The reference is Orthanc 26.9.1 (core 1.13.0) with its DICOMweb plugin;
pacs-rs aims to be compatible with it. Deliberate differences are listed in the test, each with
its reason. The test runs only when `PACS_TEST_REFERENCE_URL` is set:

```bash
docker run -d --rm --name pacs-rs-reference -p 127.0.0.1:18042:8042 \
  -e DICOM_WEB_PLUGIN_ENABLED=true -e ORTHANC__AUTHENTICATION_ENABLED=false \
  orthancteam/orthanc:26.9.1
PACS_TEST_REFERENCE_URL=http://127.0.0.1:18042/dicom-web cargo test -p pacs-server --test reference
docker stop pacs-rs-reference
```

## Benchmarks

`scripts/bench.sh` times the requests a viewer makes for one CT series; results and their limits
are in [docs/benchmarks.md](docs/benchmarks.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
