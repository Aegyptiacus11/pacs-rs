# Roadmap

The way from today's pacs-rs to one that can run in production, first as a **research PACS**
beside a hospital's clinical PACS, then as a **clinical PACS** itself: both in one program
([the role](interop.md#the-role)). The milestones are in the order they are meant to be built:
the first three make it safe to run with real data, the next two complete the research PACS,
the sixth adds what a hospital's own archive needs, the last one proves it. Inside a milestone
the order is free.

Every behaviour comes from the DICOM standard or an IHE profile and is tested against DCMTK,
dcm4che and pynetdicom ([sources](interop.md)). An item is done when its "done when" holds in a
test that CI runs.

## 1. Safe to expose

Built: the [login](login.md) (`PACS_AUTH`, off by default), with sessions, roles whose
permissions can be changed, users kept in the index, [tools](login.md#tools) that connect with
an access token got by the client credentials grant of IHE Internet User Authorization and
can be revoked, [limits on requests](running.md#limits-on-requests) (timeouts, and bounds on
what is served at once, for the server and for each address), [rights per
project](login.md#rights-per-project) (a user or a tool limited to some projects is answered as
if the rest were not there), [TLS to a Postgres index](running.md#tls-to-postgres) with the server's
certificate checked, [DIMSE over TLS](dimse.md#dimse-over-tls) by the profile of PS3.15 B.12, with
mutual authentication, the [audit trail](audit.md) written to a file and [sent to a
repository](audit.md#sending-to-a-repository) over TLS, kept while it cannot be reached, and the sign-in page and the settings (users, roles, tools, account) of the
[web interface](web-interface.md). [Signing in through the hospital's OpenID
provider](login.md#signing-in-through-a-provider), which is then the only way in, and [tools'
tokens from that provider](login.md#tools-through-the-provider), checked with its published
keys or by introspection. Without it, anyone who reaches the server can read and delete
everything. Nothing remains of this milestone.

## 2. Safe to keep data

Built: [checksums](integrity.md), each stored file's SHA-256 in the index, checked on a
schedule, on request and from the command line, and [the store and the index
compared](integrity.md#the-store-and-the-index-compared) by one command that repairs on
request, and [upgrades](backup.md#upgrading) tested on the index every release since 0.5.2
wrote, on SQLite and Postgres. [Moves in the background](dimse.md) are kept in the index, and
a restart takes them up again or ends them saying why. [Deletion can be undone](trash.md) for
30 days. [Several servers](running.md#several-servers) share one Postgres index and one
store. [Backup and restore](backup.md), as the page says, is run and restored by CI on every
change, on SQLite with disk and on Postgres with S3. Nothing remains of this milestone.

## 3. Operable

Built: [settings in a file](settings.md#in-a-file), checked whole at start, a [readiness
check](running.md), `/readyz`, that fails when the index or the store does not answer, and [metrics](running.md#metrics) for Prometheus: requests, associations,
instances stored, storage used and errors, and the [Server page](web-interface.md) of the web
interface: associations as they open and close, jobs, storage, the quarantine and the last
errors, and a [deployment guide](deployment.md): Postgres and S3 from nothing, and sizing.
Nothing remains of this milestone.

## 4. Complete query and retrieve

So that workstations, viewers and other archives can use pacs-rs as they use any PACS.

Built: [C-FIND as SCP](dimse.md#being-searched) on the Patient Root and Study Root models
of PS3.4 C.6, which DCMTK's `findscu` and pynetdicom search at every level, with sequence matching (PS3.4
C.2.2.2.6) added to the [one matcher](dicomweb.md#searching) it shares with QIDO-RS and
`/tools/find`; QIDO-RS names a key of a sequence's item as `Sequence.Attribute`. [C-MOVE and C-GET as SCP](dimse.md#being-retrieved-from), which `movescu`,
`getscu` and pynetdicom retrieve with: a C-MOVE sends to the AEs listed as destinations
(`PACS_DIMSE_MOVE_DESTINATIONS`) and a move to any other is refused with status A801; a
C-GET sends on the association that asks, by SCP/SCU role selection. Both are on the two
models, and send with a C-STORE SCU that decodes for a peer that does not take the stored
transfer syntax. As SCU, [C-FIND, C-MOVE and C-GET](dimse.md) on both models: an archive of
Patient Root only is searched patients first, and a C-MOVE the archive refuses gives way
to a C-GET. Studies are [sent](dimse.md) to other AEs from the REST API, and
[forwarded by rule](dimse.md#forwarding-by-rule) as they arrive. The SCP dispatches by a
registry of services, and each AE title can offer its own services and transfer syntaxes
on one port. Every image is stored losslessly compressed: monochrome and colour in JPEG-LS,
more than 16 bits in deflated frames. What remains:

| Item | Done when |
|---|---|
| Streamed receive | a C-STORE of a large instance is read as it arrives, with memory that does not grow with its size |
| Command-line tools | `storescu`, `storescp`, `findscu`, `movescu` and `getscu` built from the same crate, for sites without DCMTK |
| DICOMweb to the end | STOW-RS of DICOM XML metadata (`includefield`, rendered instances and frames, BulkDataURI in metadata, STOW-RS of DICOM JSON with bulk data are built) |
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

## 6. The clinical archive

What a hospital's own archive does beyond storing and serving images: the Image Manager/Image
Archive of IHE Scheduled Workflow.b ([the plan](interop.md#the-clinical-archive)).

| Item | Done when |
|---|---|
| Patient updates and orders | an HL7 v2 message over MLLP that corrects a patient or merges two changes the studies already stored, and what is served afterwards has the new values [RAD-4, -12, -13] |
| Modality Performed Procedure Step | a modality's N-CREATE and N-SET are accepted and kept, and the study shows what was performed [RAD-6, -7] |
| Storage Commitment | dcm4che's `stgcmtscu` asks for commitment of stored instances and gets the N-EVENT-REPORT that lists them, with the missing ones as failures [RAD-10] |
| Instance Availability Notification | a configured destination is told when a study becomes available [RAD-49] |
| Rejection notes | a Key Object Selection that rejects instances hides them from queries and retrieval, by the reason it gives (IOCM) |
| Retention | a study is kept for a set time by rule, and cannot be deleted before it |
| Both modes in one server | one installation stores a department's studies as received and serves a project's de-identified copy, each under its own AE title and its own rights |

## 7. Proven

| Item | Done when |
|---|---|
| Fuzzing | the parsers that read what a client sends (files, PDUs, queries) run under a fuzzer in CI without a crash |
| Load | an archive of realistic size (studies and concurrent readers named in [Benchmarks](benchmarks.md)) is ingested, searched and read within stated times |
| Soak | a server takes continuous traffic for days with flat memory |
| Security review | the login, the tokens and every route that writes are reviewed by someone else |
| A first site | one real project runs on it from pull to de-identified reading |
| A first clinical site | one department runs on it as its archive, with its modalities and workstations |
| 1.0 | the REST, DICOMweb and settings interfaces are declared stable, with a policy for changing them |

## Not planned

Being the order filler or the radiology information system: pacs-rs receives orders, it does not
create them, and the modality worklist stays with the system that schedules
([the clinical archive](interop.md#the-clinical-archive)).
