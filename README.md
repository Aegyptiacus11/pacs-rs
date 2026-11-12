# pacs-rs

An open-source PACS written in Rust, built on the [dicom-rs](https://github.com/Enet4/dicom-rs)
crates. Its index is SQLite or Postgres; its files are on disk or in S3-compatible storage.

Status: early development. `pacs-server` serves DICOMweb and a REST API over HTTP, with files on
disk or in S3 and a SQLite or Postgres index, a web interface with a viewer, and speaks DIMSE:
C-ECHO, C-STORE into the archive, and C-FIND and C-MOVE to search another archive's studies and
pull them in, de-identified on the way in when a project asks for it.

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

The client's preference decides what a retrieve sends (PS3.18 8.7.3.5): the Accept header's
ranges by quality, those of equal quality in the order sent. Within the first range that takes
anything, the content as stored wins (a viewer asking for `transfer-syntax=*` gets the stored
JPEG-LS untouched), then these, in this order; nothing else can be made, so anything else is 406.
So one header asks for JPEG-LS where the server can make it and anything otherwise:
`multipart/related; type="image/jls", multipart/related; type="application/octet-stream"; transfer-syntax=*`.

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

### Elements in UN, wrong lengths and costly files

A file re-encoded from Implicit VR may carry standard elements as UN, which PS3.5 6.2.2 allows: an
undefined-length UN holds a sequence in Implicit VR Little Endian whatever the transfer syntax,
and any other UN is little endian. Every read goes through `pacs_core::wire::checked`, which gives
dicom-rs such elements under their dictionary VR, as DCMTK and dcm4che read them. The stored file
stays as received (a native one is written anew when compressed, then under the real VRs). UN stays
UN for private elements with a defined length, for tags the dictionary does not have, and for
values too long for the VR's 16-bit length or of a size it does not come in.

A value declared longer than the file, or nesting deeper than 64 sequences, is refused before
dicom-rs allocates for it: 400 on `POST /instances`, a failed part in STOW-RS, a failure status in
C-STORE. A last fragment of encapsulated pixel data cut short stays readable, as it was.

Some well-formed files take far more memory to read than their size: an 8 MB file of one-byte text
values takes 1.3 GB with its metadata JSON, while pixel data takes its own size
(`docs/benchmarks.md`). The same walk counts elements, items and text values into an estimate. A
file estimated over `PACS_MAX_READ_BYTES` is refused unread (413 on `POST /instances`, failure
reason A700 in STOW-RS, status A702 in C-STORE), and files being read share that amount, so that
some at once wait their turn instead of adding up. It is an estimate for one file at its worst:
allow about 1.6 times it, so keep it near half of the memory the server has. The default lets through about
3 million text values or 600 000 elements; raise it for larger structure sets or enhanced
multi-frame objects if the memory is there. Bodies are not counted, since they are in memory before
the walk: `PACS_MAX_UPLOAD_BYTES` and concurrent clients bound those. Identifiers in C-FIND and
C-MOVE responses go through the same walk, with a fixed limit of 64 MiB.

### Browser caching

Retrieves, frames and metadata carry a weak `ETag` and `Vary: Accept`; a request whose
`If-None-Match` holds the tag gets 304 Not Modified before any file is read. The tag is a hash of
the instances' storage keys, sizes and receipt times, the Accept header and the server's version
(metadata: of the body). One instance and its frames cannot change while instances are never
overwritten, so they are `private, max-age=86400`: a study opened again within a day costs no
request at all. Studies, series (they gain instances) and metadata, and everything when
`PACS_OVERWRITE_INSTANCES` is set, are `private, no-cache`: asked about each time, at the cost of
a round trip, not the bytes. `private` keeps them out of shared caches, since the deployment in
front decides who may read a study; a browser keeps for up to a day what it has already read.

On a 160-slice 512x512 CT stored uncompressed, one open with the header above: 83.9 MB as
stored from v0.4.0, 31.6 MB of JPEG-LS from v0.5.0, and opened again with the tags, 160 answers
of 304 and no pixel bytes.

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
| `POST /modalities/{id}/move` (`{"StudyInstanceUID": "..."}` or a list) | that AE sends the studies to pacs-rs by C-MOVE, or with `"Project"` to a project's AE title to be de-identified; the final status and counts, or with `"Asynchronous": true` a job |
| `GET /jobs`, `GET /jobs/{id}`, `POST /jobs/{id}/cancel` | the moves running in the background: state, progress and counts as they come; cancel sends C-CANCEL |

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

Off unless `PACS_DIMSE_LISTEN` names an address. What works now, steps 1 to 4 of
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
- **C-MOVE SCU**, Study Root at study level, through `POST /modalities/{id}/move`: the remote AE
  is asked to send one or more studies to pacs-rs's own AE title, never elsewhere, and they arrive
  through the C-STORE SCP below. The answer, once the move is over, has the final status
  (`Success`, or `Warning` with the failed instances named) and the Completed, Failed and Warning
  counts. 502 when the remote AE fails, saying when it does not know pacs-rs as a destination
  (status A801: register pacs-rs's AE title, host and port there); 409 when pacs-rs is not
  listening for DIMSE; a `Warning` header when the remote AE sent nothing, as it does for a
  study it does not have. With `"Asynchronous": true` it answers 202 at once with a job:
  `GET /jobs/{id}` shows `State` (`Running`, `Success`, `Warning`, `Cancelled`, `Failure`),
  `Progress` in percent and the Remaining, Completed, Failed and Warning counts of the latest
  Pending response; `POST /jobs/{id}/cancel` sends C-CANCEL-MOVE-RQ with the next one. Jobs are
  kept in memory, the latest hundred finished ones with them; a restart forgets them.
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
both ways (see "Building"); a study pulled from dcmqrscp by C-MOVE shows in `/ui/`, and one pulled
into a project shows there de-identified. Not yet: Patient Root, C-GET, TLS, C-STORE SCU. The order is
in [docs/interop.md](docs/interop.md).

## De-identification

Off unless `PACS_PROJECTS` names a file of projects. A project is an AE title of its own and a
profile: whatever is sent to that AE title, pushed or pulled (`"Project"` in a move), is
de-identified on the way in, and only the result is stored, indexed and served. The identified
instance is held in memory meanwhile, never written. Everything else is stored as received.

```json
[{"ae_title": "LIVER_2026",
  "profile": {"name": "liver-2026", "pseudonym_prefix": "LIV-",
              "options": ["retain-longitudinal-modified-dates", "retain-patient-characteristics",
                          "clean-descriptors"],
              "overrides": [{"tag": "(0019,xxxx)", "action": "K", "reason": "GE acquisition values"}],
              "date_shift_days": [-1095, -1],
              "extra_identifiers": ["Mustapha"]}}]
```

- **Profile**: PS3.15 Table E.1-1 (2026d) whole, sequences included, with the options chosen:
  `retain-uids`, `retain-device-identity`, `retain-institution-identity`,
  `retain-patient-characteristics` (Patient's Age added from the dates), `retain-longitudinal-full-dates`,
  `retain-longitudinal-modified-dates` (one shift per patient, every date, times kept) and
  `clean-descriptors` (text kept unless it holds the patient's names or identifiers, the people and
  institution, or `extra_identifiers`). `overrides` change any attribute's action, private ones included;
  `remove_when_allowed` takes X wherever the table allows it. A profile pacs-rs cannot apply is refused at
  start.
- **Replacement values**: UIDs, pseudonyms (Patient ID and Name) and date shifts are derived with HMAC from
  the project's secret, in `PACS_PROJECT_SECRET_<AE title>` (or `..._FILE`), at least 32 bytes
  (`openssl rand -hex 32`): the same patient and UIDs map the same way on every pull, and studies, series and
  references stay linked. Keep the secret as safe as the data.
- **Recorded**: Patient Identity Removed YES, the De-identification Method Code Sequence (CID 7050) and a
  De-identification Method naming the profile and each override, new File Meta Information.
- **Quarantine**: what may carry identification pacs-rs cannot remove is not de-identified: Secondary
  Capture, encapsulated documents, SR, presentation states, ultrasound, Burned In Annotation YES, Image Type
  SCREEN SAVE (each profile may change the list). It is kept as received in `PACS_QUARANTINE`, never indexed
  or served, or without it refused (C-STORE status C003, with the reason).

Not yet: a re-identification key (the Encrypted Attributes Sequence of PS3.15 E.1.1, or an encrypted
table), Retain Safe Private, Clean Structured Content, Clean Graphics, Clean Pixel Data, a review of the
quarantine in the web interface, and a run on the MIDI-B validation data. The plan is in
[docs/interop.md](docs/interop.md).

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
  last received). The footer says how much of the list is shown (1–50
  of 1,234 studies), takes 25, 50 (the default), 100 or 200 rows a page, and goes to the
  first, previous, next or last page, a numbered one, or one typed in; ↑ ↓ go on to the next or previous page. The server filters, sorts, counts and pages the list
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
| `PACS_S3_ENDPOINT` | AWS | e.g. `http://localhost:8333` for SeaweedFS |
| `PACS_S3_REGION` | `us-east-1` | |
| `PACS_S3_ACCESS_KEY_ID`, `PACS_S3_SECRET_ACCESS_KEY` | | environment only, or `..._FILE` naming a file |
| `PACS_OVERWRITE_INSTANCES` | `false` | replace an instance sent again, instead of keeping the first copy |
| `PACS_COMPRESSION` | `jpeg-ls` | how uncompressed pixel data is stored: `jpeg-ls` (lossless, checked) or `off` |
| `PACS_MAX_UPLOAD_BYTES` | 1 GiB | largest STOW-RS or upload body |
| `PACS_MAX_READ_BYTES` | 512 MiB | memory that reading files may take at once beyond the files themselves, by estimate (see "Elements in UN, wrong lengths and costly files") |
| `PACS_CORS_ORIGINS` | none | comma-separated origins, or `*` |
| `PACS_UI` | `true` | serve the web interface at `/ui/` |
| `PACS_DIMSE_LISTEN` | off | address for DIMSE, e.g. `0.0.0.0:11112` |
| `PACS_DIMSE_AE_TITLE` | `PACSRS` | pacs-rs's AE title, called and calling |
| `PACS_DIMSE_EXTRA_AE_TITLES` | none | further called AE titles accepted, comma-separated |
| `PACS_DIMSE_REMOTES` | none | remote AEs, comma-separated, each `id=AE_TITLE@host:port` |
| `PACS_DIMSE_KNOWN_CALLERS_ONLY` | `false` | accept associations only from the remotes' AE titles |
| `PACS_DIMSE_MAX_ASSOCIATIONS` | `16` | associations served at once |
| `PACS_DIMSE_TIMEOUT` | `30` | seconds to wait for a remote AE |
| `PACS_DIMSE_MOVE_TIMEOUT` | `600` | seconds to wait between two C-MOVE responses, while a large study is sent |
| `PACS_PROJECTS` | none | a JSON file of projects, each an AE title and a de-identification profile (see "De-identification") |
| `PACS_PROJECT_SECRET_<AE title>` | | a project's secret; environment only, or `..._FILE` naming a file |
| `PACS_QUARANTINE` | none | `file:<directory>` or `s3://<bucket>[/<prefix>]` for quarantined instances; without it they are refused |
| `RUST_LOG` | `info` | log filter |

In Docker, `docker build -t pacs-server .`: a static binary on an empty image, about 20 MB, run as
an unprivileged user (uid 10001). It listens on `0.0.0.0:8080` and keeps the index and files in
the `/data` volume unless `PACS_STORAGE` says otherwise. There is no shell or curl inside, so the
image's health check is the binary itself: `pacs-server --healthcheck` asks `/healthz` on the
address of `PACS_LISTEN` and exits 0 or 1. For DIMSE, set `PACS_DIMSE_LISTEN=0.0.0.0:11112` and
publish that port.

A synthetic series to try it with:

```bash
cargo run -p pacs-core --features synthetic --example write_synthetic -- synthetic 1 3
curl -X POST --data-binary @synthetic/00001.dcm http://127.0.0.1:8080/instances
```

## Releases

A release is a tag `v<version>` matching the workspace version in `Cargo.toml`:

```bash
git tag -a v0.5.2 -m v0.5.2 && git push origin v0.5.2
```

The tag builds nothing here: this repository has no CI of its own. The Docker image is
built from the `Dockerfile` (see [Building](#building)) by whoever deploys it.

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
