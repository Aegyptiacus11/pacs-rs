# DIMSE

New to the protocol? [DICOM: the network](dicom-network.md) explains the words used here.

Off unless `PACS_DIMSE_LISTEN` names an address. What works now is steps 1 to 4 of
[the plan for hospital interfaces](interop.md), a research PACS that pulls studies from a
hospital's PACS and de-identifies them on ingest, and the first part of step 8: other
applications can search pacs-rs.

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
- **C-FIND SCP**, Study Root, at the STUDY, SERIES and IMAGE levels: [Being
  searched](#being-searched) below.
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
| `0211` Unrecognized operation | an operation pacs-rs does not provide (C-MOVE, C-GET for now), or one the SOP class does not have |

Tested against DCMTK 3.6.9 (`echoscu`, `storescu`, `storescp`, `dcmqrscp`, `findscu`) and pynetdicom 3.0.4,
both ways (see [Building](building.md)); a study pulled from dcmqrscp by C-MOVE shows in `/ui/`, and one pulled
into a project shows there de-identified. Not yet: retrieving from pacs-rs (C-MOVE and C-GET
as SCP), Patient Root, C-GET as SCU. The order is in [the hospital interfaces
plan](interop.md).

## Sending

What a retrieval from pacs-rs will send with is built, and nothing sends with it yet: the
C-STORE SCU (`pacs_dimse::Sender`), and the list of AEs it may send to.

- **Destinations**: `PACS_DIMSE_MOVE_DESTINATIONS` names, by their `id`, the remote AEs of
  `PACS_DIMSE_REMOTES` that studies may be sent to. None unless named: the hospital's
  archive is among the remote AEs, and a research PACS only reads from it. A name that is
  not a remote AE, or two destinations with one AE title, stops the start.
- **Transfer syntax**: for each SOP class pacs-rs proposes the transfer syntax the instance
  is stored in, and Explicit and Implicit VR Little Endian. A peer that takes the stored one
  gets the data set as stored, byte for byte; one that takes only uncompressed ones, as
  DCMTK's `storescp` does unless told otherwise, gets the pixel data decoded. What the
  codecs do not decode (colour, more than 16 bits) and the peer does not take as stored is
  not sent, and says so.
- **An instance whose SOP class the peer accepted no presentation context for is not sent**,
  and the others are: a failed sub-operation, as PS3.4 C.4.2.3.1 has it.
- A C-STORE that is the sub-operation of a move carries the Move Originator AE title and
  Message ID (PS3.7 9.3.1.1).

Tested against DCMTK 3.6.9 `storescp` (uncompressed only, and preferring JPEG-LS: the files
it writes hold the original pixels either way, checked with `dcmdjpls`) and a pynetdicom
3.0.4 Storage SCP that takes Implicit VR Little Endian only.

## Being searched

A viewer or a workstation searches pacs-rs with C-FIND on the Study Root Query/Retrieve
Information Model (PS3.4 C.6.2), as it searches any archive:

```bash
findscu -S -aet VIEWER -aec PACSRS -k QueryRetrieveLevel=STUDY \
  -k "PatientName=DOE*" -k StudyDate=20260101-20260131 -k StudyInstanceUID pacs.example 11112
```

- **Who is answered**: the AE titles of `PACS_DIMSE_REMOTES`. Another caller gets `0124`
  Refused: Not authorized, and can still send and echo. An AE title is a claim, not a
  proof: who may connect at all is for [TLS](#dimse-over-tls) with client certificates to
  settle.
- **What is seen** depends on the AE title that is called, since DIMSE has no accounts.
  pacs-rs's own AE title (and the extra ones) shows every study. A
  [project's](de-identification.md) AE title shows the studies of that project and no
  other, as an account [limited to that project](login.md#rights-per-project) sees them
  over HTTP: a study outside it is not found, at any level.
- **The search** is the hierarchical one (PS3.4 C.4.1.3.1.1): studies; a study's series,
  with its Study Instance UID; a series' instances, with both UIDs. Relational queries are
  not negotiated.
- **Matching** is the same as [QIDO-RS](dicomweb.md#searching) does, by the same code
  (PS3.4 C.2.2.2): single value, list of UID, universal, wild card (`*`, `?`), range, and
  sequence matching. Names match without regard to case, the rest with; dates and times by
  their meaning. A range of dates and a range of times are read apart. Empty value and
  multiple value matching are not negotiated: two values in a key that is not a UID are
  refused.
- **One Pending response per match**, each with the keys that were asked for, Query/Retrieve
  Level, the Unique Keys of the levels above, and Retrieve AE Title, which is the AE title
  that was called. A key the entity has no value for comes back empty. Text is answered in
  UTF-8 (Specific Character Set `ISO_IR 192`) when it is not plain ASCII, whatever the
  files were written in.
- **Studies are read from a cursor over the index**, a page of rows at a time, so a search
  that matches every study of a large archive costs no more memory than a small one. A
  study's series and a series' instances are read whole, then matched.
- **C-CANCEL-FIND-RQ** is looked for between two responses, and ends the search with status
  Cancel.
- **The [audit trail](audit.md#what-is-recorded)** has each search as a Query, with the
  Identifier as it was sent.

| Level | Keys |
|---|---|
| STUDY | Study Instance UID, Study Date, Study Time, Accession Number, Patient's Name, Patient ID, Study ID; and Modalities in Study, Referring Physician's Name, Study Description, Patient's Birth Date, Patient's Sex, Number of Study Related Series, Number of Study Related Instances |
| SERIES | Series Instance UID, Modality, Series Number; and Series Description, Number of Series Related Instances, Performed Procedure Step Start Date and Time, Request Attributes Sequence (Scheduled Procedure Step ID, Requested Procedure ID) |
| IMAGE | SOP Instance UID, Instance Number; and SOP Class UID, Number of Frames, Rows, Columns, Bits Allocated |

Each level has its Unique and Required Keys (PS3.4 Tables C.6-5, C.6-3 and C.6-4) and the
Optional ones listed after "and". Any other key is left out of the responses, which then
have status `FF01` instead of `FF00`: the caller filters on it itself (PS3.4 C.4.1.2.1).

C-FIND statuses (PS3.4 Table C.4-1, PS3.7 9.1.2.1.6); a Failure has an Error Comment, and
names the Offending Element when there is one:

| Status | When |
|---|---|
| `FF00` Pending | a match, every key asked for supported |
| `FF01` Pending | a match; a key that was asked for is not one pacs-rs has at this level |
| `0000` Success | every match has been sent; alone when nothing matches |
| `FE00` Cancel | the search was cancelled |
| `0124` Refused: Not authorized | the calling AE title is not one of `PACS_DIMSE_REMOTES` |
| `A700` Refused: Out of resources | the Identifier is larger than `PACS_MAX_UPLOAD_BYTES` |
| `A900` Data Set does not match SOP Class | no Query/Retrieve Level, or one that is not STUDY, SERIES or IMAGE; no single value in the Unique Key of a level above; no Identifier |
| `C000` Unable to process | the Identifier cannot be read; a value that is not a match for its key (a date that is not one, a wild card in a UID, two values, a range of times across midnight); a sequence key of several items |
| `C001` Unable to process | the index could not be searched |

## DIMSE over TLS

Without it, everything on the DIMSE port travels in clear: patient names, images, AE titles.
pacs-rs follows the BCP 195 RFC 8996, 9325 TLS Secure Transport Connection Profile
(PS3.15 B.12).

| What B.12 asks | pacs-rs |
|---|---|
| TLS 1.2 supported, TLS 1.3 preferred, nothing older | TLS 1.2 and 1.3 only; 1.3 when the peer offers it |
| no cipher suite without encryption or integrity | none exists in its TLS library |
| strict TLS rather than an upgrade from plain | with a certificate the DIMSE port speaks TLS only; a plain caller is not answered |
| a server supports mutual authentication, and may be configured not to use it | callers must show a certificate signed by a known authority, unless `PACS_DIMSE_TLS_CLIENT_AUTH=off` |
| a TLS port for DIMSE apart from the one for DICOMweb | DIMSE has its own port (`PACS_DIMSE_LISTEN`); the standard recommends 2762 for DIMSE over TLS |

**To receive over TLS**, give pacs-rs a certificate and say who may call:

```bash
PACS_DIMSE_LISTEN=0.0.0.0:2762
PACS_DIMSE_TLS_CERT_FILE=/secrets/pacs.crt     # pacs-rs's certificate and its chain, PEM
PACS_DIMSE_TLS_KEY_FILE=/secrets/pacs.key      # its private key, PEM
PACS_DIMSE_TLS_CA_FILE=/secrets/site-ca.crt    # the authorities that sign the callers' certificates
```

**To call a remote AE over TLS**, mark it with `/tls` after its port, and name the authorities
that sign its certificate:

```bash
PACS_DIMSE_REMOTES=hospital=PACS@pacs.hospital.example:2762/tls
PACS_DIMSE_TLS_CA_FILE=/secrets/site-ca.crt
```

- The remote AE's certificate must name the host it is configured under here
  (`pacs.hospital.example`), and chain to one of the authorities. A remote AE marked `/tls`
  is never called in plain.
- pacs-rs shows its own certificate to a remote AE that asks for one, when it has one.
- A remote AE without `/tls` is called in plain, as before: the two kinds can be mixed.
- **A C-MOVE has two connections.** pacs-rs calls the archive, and the archive calls pacs-rs
  back to send the images. With a certificate on pacs-rs, that second connection is TLS
  too, so the archive must be able to send over TLS, to pacs-rs's TLS port.
- **A certificate says which machine, not which AE title.** Any caller with a certificate
  from one of the authorities is taken, under whatever AE title it claims that pacs-rs
  accepts. Give `PACS_DIMSE_TLS_CA_FILE` an authority that signs only the systems allowed
  to connect.
- A start whose TLS settings do not add up stops with the reason: a certificate without
  its key, a listener that asks for certificates with no authority to check them by, a
  `/tls` remote AE with no authority, a file that cannot be read.

Tested between two pacs-rs, with certificates made in the test: every combination above is
accepted or refused as said. Checked by hand against pynetdicom over Python's OpenSSL, on
TLS 1.2 and on TLS 1.3: C-ECHO and C-STORE with a client certificate succeed, and a caller
without one, or in plain, is refused.

