# pacs-rs

An open-source PACS written in Rust, built on the [dicom-rs](https://github.com/Enet4/dicom-rs)
crates.

Status: early development, nothing usable yet.

## First milestone

A DICOMweb server that a web viewer can read CT series from:

- QIDO-RS: search studies, series and instances, with `includefield` for
  NumberOfStudyRelatedInstances (0020,1208) and NumberOfSeriesRelatedInstances (0020,1209);
- WADO-RS: a series' metadata in one response, an instance's frames and the instance itself
  (`multipart/related`, in the stored transfer syntax);
- STOW-RS, plus a plain `POST /instances` that takes one DICOM file in the body;
- DICOM files kept in S3-compatible object storage (SeaweedFS, MinIO, R2) or on local disk,
  with a small index in SQLite.

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
