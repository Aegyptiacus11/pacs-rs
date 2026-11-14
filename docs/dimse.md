# DIMSE

Off unless `PACS_DIMSE_LISTEN` names an address. What works now is steps 1 to 4 of
[the plan for hospital interfaces](interop.md), a research PACS that pulls studies from a
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
both ways (see [Building](building.md)); a study pulled from dcmqrscp by C-MOVE shows in `/ui/`, and one pulled
into a project shows there de-identified. Not yet: Patient Root, C-GET, TLS, C-STORE SCU. The order is
in [the hospital interfaces plan](interop.md).
