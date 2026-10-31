# pacs-rs

An open-source PACS written in Rust, built on the [dicom-rs](https://github.com/Enet4/dicom-rs)
crates. Its index is SQLite or Postgres; its files are on disk or in S3-compatible storage.

Status: early development. `pacs-server` serves DICOMweb over HTTP, with files on disk or in
S3 and a SQLite index, and speaks DIMSE: C-ECHO, C-STORE into the archive, and C-FIND to search
another archive's studies.

## Crates

| Crate | What |
|---|---|
| `pacs-core` | parse a DICOM file, its UIDs, per-level attributes, DICOM JSON, frames; lossless codecs and transcoding |
| `pacs-store` | the files, as received: local disk, S3-compatible storage or memory |
| `pacs-index` | the index, SQLite or Postgres: hierarchy, counts, attributes, metadata, storage keys |
| `pacs-archive` | ingest (compress, store, then index, undone if the index refuses) and deletion (index then files) |
| `pacs-dicomweb` | QIDO-RS, WADO-RS, STOW-RS over axum |
| `pacs-rest` | the REST API: resources by public ID, uploads, deletion, find, archives |
| `pacs-dimse` | DIMSE on dicom-ul: C-ECHO SCP and SCU, C-STORE SCP into the archive, C-FIND SCU |
| `pacs-server` | the binary: settings, `/healthz`, the REST API at `/`, DICOMweb under `/dicom-web`, the web interface at `/ui/` |

`ui/` is the web interface, a Vite project built into the binary (see "Web interface").

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
| `GET /tools/create-archive?resources=a,b`, `POST /tools/create-archive` (`{"Resources": [...]}`) | patients, studies, series and instances in one ZIP, laid out as above, each file once; a series or instance keeps its number in the series |
| `POST /tools/find` | `Level` Patient, Study, Series or Instance, exact matching on the UIDs, PatientID, AccessionNumber, StudyDate |
| `GET /modalities[?expand]` | the remote AEs of `PACS_DIMSE_REMOTES`: their ids, or each one's `AET`, `Host` and `Port` |
| `POST /modalities/{id}/echo` | a C-ECHO to that AE: 200 with `Status` and `ElapsedMs` when it answers Success, 502 with the reason when not |
| `GET /modalities/{id}/studies?PatientID=&PatientName=&AccessionNumber=&StudyDate=&ModalitiesInStudy=&limit=` | that AE's studies, by C-FIND (see "DIMSE"), as a DICOM JSON array |

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

## DIMSE

Off unless `PACS_DIMSE_LISTEN` names an address. What works now, steps 1 to 3 of
[docs/interop.md](docs/interop.md), the plan for a research PACS that pulls studies from a
hospital's PACS and de-identifies them on ingest:

- **C-ECHO SCP** on the Verification SOP Class, and **C-ECHO SCU** to the AEs of
  `PACS_DIMSE_REMOTES`, through `POST /modalities/{id}/echo`, to test a hospital link.
- **C-FIND SCU**, Study Root at study level, through `GET /modalities/{id}/studies`: by Patient
  ID, Patient's Name and Accession Number (exact, or with `*` and `?`), Study Date (a date,
  `20260101-20260131`, `-20260131` or `20260101-`) and one modality; Specific Character Set
  ISO_IR 192 when a value is not ASCII. The matches come back as DICOM JSON, as a QIDO-RS search
  answers; pacs-rs checks each against the query as well, since an archive may not match on
  every key (PS3.4 C.4.1.2.1). Past `limit` (100 unless given, 1000 at most) it cancels the
  search with C-CANCEL and a `Warning` header says more studies match; one also says when the
  archive did not support a key or ended the search early. A query that breaks the matching
  rules of PS3.4 C.2.2.2 (two values, a malformed date, an unknown parameter) is 400.
- **C-STORE SCP** for the 173 Storage SOP Classes of PS3.4 Table B.5-1 (DICOM 2026d), in any
  transfer syntax dicom-rs reads. Each data set is kept in the transfer syntax it came in, with
  file meta information added (PS3.10 7.1, the sending and receiving AE titles included), and
  goes through the same ingest as `POST /instances`: compressed as `PACS_COMPRESSION` says,
  stored, indexed; an instance sent again is kept as first stored unless
  `PACS_OVERWRITE_INSTANCES` is set, and answered Success either way.
- **AE titles**: `PACS_DIMSE_AE_TITLE` and any `PACS_DIMSE_EXTRA_AE_TITLES` are accepted on one
  port with one set of presentation contexts; another called AE title is rejected (PS3.8 reason
  7), and with `PACS_DIMSE_KNOWN_CALLERS_ONLY` so is a calling AE title not among the remotes
  (reason 3). Over `PACS_DIMSE_MAX_ASSOCIATIONS`, an association is rejected as transient,
  local limit exceeded, for the sender to retry.

C-STORE statuses (PS3.4 Table B.2-1, PS3.7 9.1.1.1.9), each with an Error Comment:

| Status | When |
|---|---|
| `0000` Success | stored, or already stored |
| `A700`, `A701` Refused: Out of resources | the store, or the index, did not take it |
| `A702` Refused: Out of resources | the data set is larger than `PACS_MAX_UPLOAD_BYTES` |
| `A900` Data Set does not match SOP Class | SOP Class UID is not the Affected SOP Class UID |
| `C000` Cannot understand | the data set cannot be read, or lacks a UID an instance needs |
| `C001` Cannot understand | SOP Instance UID is not the Affected SOP Instance UID |
| `C002` Cannot understand | the UIDs contradict the archive (an instance already in another series) |
| `0122` SOP Class not supported | a SOP class pacs-rs does not have |
| `0212` Mistyped argument | a SOP class not the one agreed for the presentation context |
| `0211` Unrecognized operation | an operation pacs-rs does not provide (C-FIND, C-MOVE, C-GET for now) |

Tested against DCMTK 3.6.9 (`echoscu`, `storescu`, `storescp`, `dcmqrscp`) and pynetdicom 3.0.4,
both ways (see "Building"). Not yet: Patient Root queries, C-MOVE, de-identification on ingest,
TLS, C-STORE SCU; the order is in [docs/interop.md](docs/interop.md).

## Web interface

At `/ui/` (`/` redirects there), on the same REST API and DICOMweb as any other client. Built with
React, [shadcn/ui](https://ui.shadcn.com/) (Radix primitives, Tailwind) and lucide icons; the
viewer is [Cornerstone3D](https://www.cornerstonejs.org/) 5.14.0.

- **Layout**: a rail down the left side to go places (Studies, Patients), a bar along the top
  for the tools (the search, light or dark, a warning when the server doesn't answer), the page
  in between.
- **Studies**: a worklist and, beside it, the chosen study. One row of filters over the table:
  a filter box (`/`), the date, the modality (every DICOM imaging modality), the sex and the age
  at the study (0–17, 18–39, 40–64, 65+), each with how many studies it holds. The date opens
  a small panel: today, yesterday, this week, this month and this year, each with its studies
  (clicked again, it lets go), and From
  and To fields, typed (in the browser's day, month, year order, or year first) or picked on a
  small calendar under each (the days with studies dotted). Sortable columns (date first, then patient, study,
  modality, series, images). A click on a row shows the
  study in the pane: the patient, its series as thumbnails (a click opens one), the patient's
  other studies (prior or later), its details, and the actions: open, download (ZIP), and under
  ⋯ copy the Study UID, the patient's studies, download everything of the patient, delete the
  study or the patient. A double click or Enter opens it; ↑ ↓
  move through the list. With no study chosen, the pane sums up what the list holds, for the
  dates and filters chosen (patients, studies, series, images, studies by modality, the
  last received). Fifty studies a page: the footer says how many the list holds (1–50
  of 1,234 studies) with Previous and Next, and ↑ ↓ go on to the next or previous page. The server filters, sorts, counts and pages the list
  (`/ui/api/studies`), so the browser only ever holds one page, however large the archive. Checkboxes download or delete several
  studies at once (one ZIP from `/tools/create-archive`).
- **Search**: two, with two jobs. The top bar's (`Ctrl K`, `⌘K` on a Mac) finds a patient or a
  study anywhere in the archive and shows it on the page already open: on the studies, the
  patient's studies or the study itself; on the patients, the patient's record. Each list has
  its own box (`/`) that filters that list as words are typed and never leaves the page.
- **Patients**: a list (latest study first, by name or by number of studies, a page at a time
  from `/ui/api/patients`) and the chosen patient's record: who they are, their totals
  (studies, series, images, first study, modalities), and a timeline of their studies, newest
  first, each with its series as thumbnails. Tick two studies to compare them: the viewer shows
  each one's largest series side by side, scrolling together. Download everything of the
  patient, or delete them.
- **Light or dark**: one button on the rail, remembered in the browser; the system setting until it is used. The viewer is always dark.
- **Upload**: a dialog with a drop zone, or files and folders dropped anywhere; each file goes
  to `POST /instances` and is counted as new, already stored or skipped.
- **Viewer**: full screen and dark, laid out like a reading workstation. A toolbar of labelled
  tools: window/level with presets and invert, pan, zoom, a magnifying glass (held over a spot, 1.5 to 8 times with the wheel), measurements
  (length, pixel value, angle, Cobb angle, bidirectional, elliptical and rectangle ROI), rotate
  and flip, Add view (the active series in the next plane, sagittal then coronal, or the next
  series), Views (up to 2 × 3, 3 planes, scrolling the viewports together: Sync), cine, DICOM
  tags, export (PNG of the view, or the DICOM file), full screen. Each viewport has a plane menu
  (as acquired, axial, sagittal, coronal, 3D) and can be removed. A 3D view renders the
  series' volume (CT: bone, muscle and bone, vessels, lung, skin, MIP; MR: default, angio, T2
  brain, MIP); drag to turn it, the wheel or a right drag zooms. On the left, the patient's studies (current, priors and later
  ones) and their series: a click shows one in the active viewport, a drag shows it in any.
  Each viewport has its own text, orientation letters, a centimetre scale, its position in
  the series along the top, and a preset menu. MPR shows the series as a volume, axial,
  sagittal and coronal with crosshairs, and a full-thickness coronal MIP; each view takes a
  thin slice or a MIP slab of 5, 10 or 20 mm. Frames come over WADO-RS as stored and are
  decoded in the browser (JPEG-LS, JPEG, JPEG 2000, HTJ2K, JPEG XL). MPR, other planes and 3D need WebGL; the rest
  also renders without it ("Software rendering" under Views, turned on by itself when a
  browser's WebGL draws blank). Keys follow the names: W window, P pan, Z zoom, M magnifier,
  L length, A angle, C Cobb, B bidirectional, E ellipse, ⇧E rectangle, D pixel value (density),
  R rotate (⇧R left), H and V flip, I invert, 0 reset, N new view, X 3 planes, S sync; `?`
  lists them all.
- **Hanging protocols**: a study opens as the best protocol that fits it lays it out: the
  current study beside its latest prior (the prior's most similar series, scrolling linked),
  two views for CR, DX, MG and RF, the largest series, every series, or MPR. The toolbar lists
  the ones that fit, with previous and next (`,` `.`). "Save this layout as a protocol" keeps
  the arrangement by kind of series (current or prior, modality, description), not by UID, so
  it serves every matching study. Saved protocols are kept by the server, in the index, so
  every browser and everyone using the server shares them; they come first when they fit.
  Opening one series (a thumbnail) shows that series alone.
- **Measurements are kept**: each one is saved with its study as soon as it is drawn, changed
  or removed, and is there again when the study is opened, in any browser. Clear removes them
  all, with an undo.

The shared protocols are at `/ui/api/protocols`: `GET` lists them (most recently saved first),
`PUT /ui/api/protocols/{id}` keeps one (a JSON object whose `id` is the path's, at most 64 KiB;
at most 1000 protocols), `DELETE` removes one. They are the interface's own documents, stored as
received in the index's `ui_protocols` table.

The study list is at `/ui/api/studies`: `q` (words, each found in the patient's name or ID,
the accession number, the description, the referring physician or the date), `from` and `to`
(`YYYYMMDD`, included), `modality` and `sex` (comma-separated; `O` for other), `age` (the age at the study, ranges of
years: `0-17,65-`), `patient` (a
public ID), `sort` (`patient`, `description`, `modality`, `date`, `series`, `images`,
`received`), `order` (`asc`, `desc`), `offset`, `limit` (at most 1000) and `summary=true` (the
whole list's totals and its last received studies). It answers `{"Total", "Studies",
"Summary"?}`; `/ui/api/studies/facets[?patient=]` gives the filters' choices with their counts.
`/ui/api/patients` lists patients the same way (`q`, `sort`: `name`, `last` or `studies`,
`order`, `offset`, `limit`), each with its study and image counts, modalities and first and last
study dates.

The measurements are at `/ui/api/annotations/{StudyInstanceUID}`: `GET` returns the study's list
(a JSON array, empty when it has none), `PUT` replaces it (an array of objects, at most 1 MiB; an
empty one removes it), for a study the server has. They are Cornerstone3D annotations, stored as
received in the `ui_annotations` table; deleting the study deletes them.

Like the REST API, it has no login yet: whoever reaches it can delete, and change the shared
protocols and the measurements. `PACS_UI=false` turns it off, protocols and measurements included.

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
| `PACS_UI` | `true` | serve the web interface at `/ui/` |
| `PACS_DIMSE_LISTEN` | off | address for DIMSE, e.g. `0.0.0.0:11112` |
| `PACS_DIMSE_AE_TITLE` | `PACSRS` | pacs-rs's AE title, called and calling |
| `PACS_DIMSE_EXTRA_AE_TITLES` | none | further called AE titles accepted, comma-separated |
| `PACS_DIMSE_REMOTES` | none | remote AEs, comma-separated, each `id=AE_TITLE@host:port` |
| `PACS_DIMSE_KNOWN_CALLERS_ONLY` | `false` | accept associations only from the remotes' AE titles |
| `PACS_DIMSE_MAX_ASSOCIATIONS` | `16` | associations served at once |
| `PACS_DIMSE_TIMEOUT` | `30` | seconds to wait for a remote AE |
| `RUST_LOG` | `info` | log filter |

In Docker, `docker build -t pacs-server .`: a static binary on an empty image, about 20 MB, run as
an unprivileged user (uid 10001). It listens on `0.0.0.0:8080` and keeps the index and files in
the `/data` volume unless `PACS_STORAGE` says otherwise. With no shell or curl inside, health
checks call `/healthz` from outside. For DIMSE, set `PACS_DIMSE_LISTEN=0.0.0.0:11112` and publish
that port.

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
