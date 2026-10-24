# pacs-rs

An open-source PACS written in Rust, built on the [dicom-rs](https://github.com/Enet4/dicom-rs)
crates.

Status: early development. The DICOMweb layer works as a library and is tested; there is
no server binary yet.

## Crates

| Crate | What |
|---|---|
| `pacs-core` | parse a DICOM file, its UIDs, per-level attributes, DICOM JSON, frames |
| `pacs-store` | the files, as received: local disk, S3-compatible storage or memory |
| `pacs-index` | the SQLite index: hierarchy, counts, attributes, metadata, storage keys |
| `pacs-archive` | ingest: store then index, undone if the index refuses |
| `pacs-dicomweb` | QIDO-RS, WADO-RS, STOW-RS over axum |

## DICOMweb

| Request | |
|---|---|
| `GET /studies`, `/studies/{study}/series`, `.../series/{series}/instances` | QIDO-RS; counts, Modalities in Study and Retrieve URL always included |
| `GET /studies/{study}`, `.../series/{series}`, `.../instances/{instance}` | WADO-RS retrieve, `multipart/related; type="application/dicom"`, as stored |
| `GET .../metadata` (study, series, instance) | DICOM JSON without pixel data |
| `GET .../instances/{instance}/frames/{list}` | frames in the stored transfer syntax: `application/octet-stream` or the compressed type of PS3.18 Table 8.7.3-5 |
| `POST /studies`, `POST /studies/{study}` | STOW-RS, `application/dicom` parts |
| `POST /instances` | one DICOM file as the body; answers with its UIDs |

Not yet: wildcard, range, list and fuzzy matching (answered with 400, not ignored);
`includefield` for attributes beyond the stored levels; transcoding (406 when the client wants
a transfer syntax other than the stored one); rendered media; JSON/XML + bulk data STOW;
BulkDataURI in metadata.

DICOM JSON numbers: IS and DS values are written as JSON numbers, padding trimmed (PS3.18 Table
F.2.3-1 allows numbers or strings).

DIMSE services (C-ECHO, C-STORE, C-FIND, C-MOVE over dicom-ul) come after.

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

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without
any additional terms or conditions.
