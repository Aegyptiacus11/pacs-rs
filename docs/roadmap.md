# Roadmap

The way from today's pacs-rs to one that can run in production as a **research PACS** beside a
hospital's clinical PACS ([the role](interop.md#the-role)). The milestones are in the order they
are meant to be built: the first three make it safe to run with real data, the next two complete
what it does, the last one proves it. Inside a milestone the order is free.

Every behaviour comes from the DICOM standard or an IHE profile and is tested against DCMTK,
dcm4che and pynetdicom ([sources](interop.md)). An item is done when its "done when" holds in a
test that CI runs.

## 1. Safe to expose

Built: the [login](login.md) (`PACS_AUTH`, off by default), with sessions, the roles reader,
editor, manager and admin, and users kept in the index. Without it, anyone who reaches the
server can read and delete everything. What remains:

| Item | Done when |
|---|---|
| Sign-in page | the [web interface](web-interface.md) signs in, keeps its session and downloads archives with the login on; today only the [REST API](rest-api.md) and [DICOMweb](dicomweb.md) can be used with it |
| Signing in through a provider | a user signs in with OpenID Connect, and pacs-rs gives the same session as after a password |
| Rights per project | a user can be limited to some projects |
| Tokens for tools | a viewer or a pipeline connects with a token that can be revoked, following IHE Internet User Authorization |
| TLS on DIMSE | associations can require TLS, following PS3.15 H.1 (step 6 of the [plan](interop.md#build-order)) |
| TLS to Postgres | the index connection is encrypted and the server's certificate checked |
| Audit trail | every read, store, export, deletion and login is recorded as the DICOM audit message of PS3.15 (IHE ATNA), kept apart from the application log |
| Limits on requests | timeouts and a bound on concurrent requests, so that one client cannot hold the server |

## 2. Safe to keep data

| Item | Done when |
|---|---|
| Checksums | each stored file's hash is in the index, checked on a schedule and on request |
| Consistency check | one command lists files without an index entry and index entries without a file, and repairs on request |
| Backup and restore | a documented procedure for SQLite and Postgres with disk and S3, restored in a test |
| Deletion that can be undone | a deleted study stays recoverable for a set time before its files go |
| Several processes on one index | uploads and deletions stay correct with two servers on one Postgres ([REST API](rest-api.md)) |
| Jobs that outlive a restart | moves in the background are in the index, and resume or fail clearly after a restart ([DIMSE](dimse.md)) |
| Upgrades | every release migrates the index of the one before, tested on a copy of each schema version |

## 3. Operable

| Item | Done when |
|---|---|
| Health that means it | a readiness check that fails when the index or the store does not answer (`/healthz` only says the process is up) |
| Metrics | requests, associations, ingest rate, storage used and errors, in Prometheus format |
| Administration page | open associations as they open and close, running jobs, storage, the quarantine and the last errors, in the web interface |
| Settings in a file | one configuration file beside the `PACS_*` variables, checked whole at start |
| Deployment guide | a page for a real installation: proxy, TLS, Postgres, S3, backup, sizing, upgrade |

## 4. Complete query and retrieve

So that workstations, viewers and other archives can use pacs-rs as they use any PACS.

| Item | Done when |
|---|---|
| One matcher | single value, list of UID, universal, wildcard, range and sequence matching of PS3.4 C.2.2.2, shared by QIDO-RS, `/tools/find` and C-FIND |
| C-FIND, C-MOVE and C-GET as SCP | DCMTK's `findscu`, `movescu` and `getscu` query and retrieve from pacs-rs (step 8 of the [plan](interop.md#rules-for-the-scp-side-step-8)) |
| Patient Root | both information models of PS3.4 C.6, as SCU and SCP |
| C-GET SCU | a study is pulled from an archive that refuses C-MOVE to an unknown destination |
| C-STORE SCU | pacs-rs sends studies to another AE, transcoding when the peer asks |
| Capabilities per AE title | each AE title has its own SOP classes and transfer syntaxes on one port |
| A service per SOP class | the SCP finds its handler for each SOP class in a registry, so a new service is added without touching the association loop |
| Streamed receive | a C-STORE of a large instance is read as it arrives, with memory that does not grow with its size |
| Destinations for C-MOVE | the AE titles pacs-rs may send to are configured, and a move to any other is refused with status A801 |
| Command-line tools | `storescu`, `storescp`, `findscu`, `movescu` and `getscu` built from the same crate, for sites without DCMTK |
| DICOMweb to the end | `includefield`, rendered media, STOW-RS with JSON or XML and bulk data, BulkDataURI in metadata |
| Lossless storage of every image | colour and more than 16 bits are compressed too, checked as today |
| Conformance Statement | the PS3.2 document, generated from what the code negotiates |

## 5. The research workflow

| Item | Done when |
|---|---|
| Re-identification key | the Encrypted Attributes Sequence of PS3.15 E.1.1, or an encrypted table, read back in a test ([De-identification](de-identification.md)) |
| The remaining options | Retain Safe Private, Clean Structured Content, Clean Graphics and Clean Pixel Data |
| Quarantine review | quarantined instances are listed, viewed and released or deleted in the web interface |
| MIDI-B | the de-identification is scored on the MIDI-B challenge's validation data |
| Write-back | segmentations as DICOM Segmentation and measurements as Structured Reports go to the hospital's destination (clinical mode, step 7) |
| Labels, metadata and modifications | the rest of the [REST API](rest-api.md) that research tools use |

## 6. Proven

| Item | Done when |
|---|---|
| Fuzzing | the parsers that read what a client sends (files, PDUs, queries) run under a fuzzer in CI without a crash |
| Load | an archive of realistic size (studies and concurrent readers named in [Benchmarks](benchmarks.md)) is ingested, searched and read within stated times |
| Soak | a server takes continuous traffic for days with flat memory |
| Security review | the login, the tokens and every route that writes are reviewed by someone else |
| A first site | one real project runs on it from pull to de-identified reading |
| 1.0 | the REST, DICOMweb and settings interfaces are declared stable, with a policy for changing them |

## Not planned

Replacing a hospital's clinical PACS: HL7 v2 orders, Modality Performed Procedure Step, Storage
Commitment and the rest of IHE Scheduled Workflow.b
([why](interop.md#not-planned-replacing-a-clinical-pacs)).
