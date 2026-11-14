# Building and testing

The toolchain is pinned in `rust-toolchain.toml`; rustup picks it up. CharLS is C++, built by
the `charls` crate and linked statically, so building also needs `cmake` and a C++ compiler.

```bash
cargo test --workspace
```

The web interface is built with Node (24 or later) and embedded when `pacs-server` is compiled
(`crates/pacs-server/build.rs`, from `ui/dist`, or from the directory in `PACS_UI_DIST`).
Without a build, the server still works and `/ui/` says how to make one. The Docker image
builds it in a stage of its own.

```bash
(cd ui && npm ci && npm run build)
cargo build --release -p pacs-server
```

For work on the interface, `npm run dev` in `ui/` serves it with hot reload and sends API
requests to a pacs-server on `PACS_DEV_SERVER` (default `http://127.0.0.1:8080`);
`npm run check` type-checks it. Components from shadcn/ui live in `ui/src/components/ui`.

On Windows, a static build (no Visual C++ runtime needed) also tells CMake which C runtime
CharLS uses; this is what CI and the release do:

```powershell
$env:RUSTFLAGS = "-C target-feature=+crt-static"
$env:CMAKE_TOOLCHAIN_FILE = "$PWD\cmake\msvc-static-crt.cmake"
cargo build --release -p pacs-server
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

The interoperability tests of DIMSE are ignored unless asked for; they need DCMTK (on `PATH`, or
in `PACS_TEST_DCMTK_BIN`) and a Python with pynetdicom (`PACS_TEST_PYTHON`, default `python3`):

```bash
cargo test -p pacs-dimse --test interop -- --ignored
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
