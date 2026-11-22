# DIMSE

New to the protocol? [DICOM: the network](dicom-network.md) explains the words used here.

Off unless `PACS_DIMSE_LISTEN` names an address. What works now is steps 1 to 4 of
[the plan for hospital interfaces](interop.md), a research PACS that pulls studies from a
hospital's PACS and de-identifies them on ingest, and step 8: other applications can search
pacs-rs and retrieve from it, on the Patient Root and the Study Root models.

- **C-ECHO SCP** on the Verification SOP Class, and **C-ECHO SCU** to the AEs of
  `PACS_DIMSE_REMOTES`, through `POST /modalities/{id}/echo`, to test a hospital link.
- **C-FIND SCU** at study level, through `GET /modalities/{id}/studies`: by Patient
  ID, Patient's Name and Accession Number (exact, or with `*` and `?`), Study Date (a date,
  `20260101-20260131`, `-20260131` or `20260101-`) and one modality; Specific Character Set
  ISO_IR 192 when a value is not ASCII. The matches come back as DICOM JSON, as a QIDO-RS search
  answers; pacs-rs checks each against the query as well, since an archive may not match on
  every key (PS3.4 C.4.1.2.1). Past `limit` (100 unless given, 1000 at most) it cancels the
  search with C-CANCEL and a `Warning` header says more studies match; one also says when the
  archive did not support a key or ended the search early. A query that breaks the matching
  rules of PS3.4 C.2.2.2 (two values, a malformed date, an unknown parameter) is 400. Both
  models are proposed, and Study Root is used when the archive takes it. An archive of Patient
  Root only is searched in two steps on one association, as its model has studies below their
  patient (PS3.4 C.6.1.1): the patients that meet Patient ID and Patient's Name, then the
  studies of each by its Patient ID, the other keys asked there; each study comes back with its
  patient's attributes.
- **C-MOVE SCU** at study level, through `POST /modalities/{id}/move`: the remote AE
  is asked to send one or more studies to pacs-rs's own AE title, never elsewhere, and they arrive
  through the C-STORE SCP below. The answer, once the move is over, has the final status
  (`Success`, or `Warning` with the failed instances named) and the Completed, Failed and Warning
  counts. Both models are proposed, Study Root used when the archive takes it; an archive of
  Patient Root only names a study under its patient (PS3.4 C.4.2.2.1), so the request gives
  the studies' `PatientID` too (a search's matches carry it), and without it the move is 400
  saying so. A `Warning` header when the remote AE sent nothing, as it does for a study it does
  not have. With `"Asynchronous": true` it answers 202 at once with a job: `GET /jobs/{id}`
  shows `State` (`Running`, `Success`, `Warning`, `Cancelled`, `Failure`), `Progress` in percent
  and the Remaining, Completed, Failed and Warning counts of the latest Pending response;
  `POST /jobs/{id}/cancel` sends C-CANCEL-MOVE-RQ (or C-CANCEL-GET-RQ) with the next one. Jobs
  are kept in the index, the latest hundred finished ones with them. A restart takes up the
  moves that were running, from the start (what had arrived stays, and comes again as already
  stored); one taken up three times, or whose remote AE is no longer configured, ends `Failure`
  with the reason in `Error`. `Restarts` counts the starts that took it up.
- **C-GET SCU**, where the C-MOVE cannot be made: when the remote AE does not know pacs-rs as a
  destination (status A801), takes no C-MOVE, or pacs-rs is not listening for DIMSE. The same
  request asks for the studies by C-GET (PS3.4 C.4.3.2) and they come back on the association
  that asks, pacs-rs being the SCP of the storage SOP classes by SCP/SCU Role Selection (PS3.7
  D.3.3.4); each instance goes through the same checks, statuses and ingest as one the C-STORE
  SCP receives, into a project's de-identification when the request names one, and the audit
  trail has them as received from the remote AE. The answer's `Method`, and a job's `Type`,
  say `C-GET`. One association carries 128 presentation contexts at most: two for the GET
  models, then the storage SOP classes a hospital holds most (CT, MR, PET, NM, CR, DX, MG, US,
  XA, secondary captures, segmentations, RT objects, structured reports, presentation states,
  encapsulated documents), the images among them proposed compressed too (JPEG-LS, JPEG,
  JPEG 2000, RLE), then the rest of PS3.4 Table B.5-1 in the uncompressed transfer syntaxes
  until none is left. An instance of a class left out, or stored in a compression its context
  was not proposed with and that the archive does not convert, is counted by the archive as a
  failed sub-operation. 502 when the remote AE fails by both, saying when it does not know
  pacs-rs as a destination (A801: register pacs-rs's AE title, host and port there, or let it
  answer C-GET).
- **C-STORE SCU** on pacs-rs's own initiative, through `POST /modalities/{id}/store` with
  `{"StudyInstanceUID": …}`: every instance of the studies sent on one association, each in
  the transfer syntax it is stored in when the remote AE takes it, decoded otherwise. Only to
  the remote AEs of `PACS_DIMSE_MOVE_DESTINATIONS` (409 for another: an archive pacs-rs only
  reads from is not written to because someone asked), and only studies the account sees (404
  for one outside its projects, as for one not there). The answer has the counts and `Method`
  `C-STORE`; 502 when the remote AE cannot be reached or stores none. With `"Asynchronous":
  true` it is a job of `Type` `C-STORE`, cancelled before the next instance and taken up again
  after a restart like a move. The audit trail has a Begin Transferring line a patient, from
  pacs-rs to the remote AE, for whoever asked.
- **C-FIND SCP**, Patient Root and Study Root, at every level of each: [Being
  searched](#being-searched) below.
- **C-MOVE SCP** and **C-GET SCP**, on both models and at the same levels, with the
  **C-STORE SCU** they send by: [Being retrieved from](#being-retrieved-from) below.
- **C-STORE SCP** for the 173 Storage SOP Classes of PS3.4 Table B.5-1 (DICOM 2026d), in any
  transfer syntax dicom-rs reads. Each data set is kept in the transfer syntax it came in, with
  file meta information added (PS3.10 7.1, the sending and receiving AE titles included), and
  goes through the same ingest as `POST /instances`: compressed as `PACS_COMPRESSION` says,
  stored, indexed; an instance sent again is kept as first stored unless
  `PACS_OVERWRITE_INSTANCES` is set, and answered Success either way.
- **AE titles**: `PACS_DIMSE_AE_TITLE` and any `PACS_DIMSE_EXTRA_AE_TITLES` are accepted on one
  port; another called AE title is rejected (PS3.8 reason 7). Each offers every service in every
  transfer syntax unless `PACS_DIMSE_CAPABILITIES` gives it its own: `RESEARCH=storage+verification/uncompressed`
  makes RESEARCH a storage destination that takes uncompressed images only, its other
  presentation contexts rejected at negotiation (PS3.8 9.3.3.2: abstract syntax, or transfer
  syntaxes, not supported) while pacs-rs's own AE title on the same port offers everything.
  The services are `verification`, `storage`, `find`, `move` and `get`, each with its SOP
  classes; the called AE title is read from the association request before it is negotiated.
  Over [TLS](#dimse-over-tls) that request is inside the session, so every service is
  negotiated and the AE title's own capabilities hold per operation instead, answered `0122`; and with `PACS_DIMSE_KNOWN_CALLERS_ONLY` so is a calling AE title not among the remotes
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
| `0211` Unrecognized operation | an operation the SOP class does not have |

Tested against DCMTK 3.6.9 (`echoscu`, `storescu`, `storescp`, `dcmqrscp`, `findscu`, `movescu`, `getscu`) and pynetdicom 3.0.4,
both ways (see [Building](building.md)); a study pulled from dcmqrscp by C-MOVE shows in `/ui/`, and one pulled
into a project shows there de-identified. The order is in [the hospital interfaces
plan](interop.md).

## Forwarding by rule

`PACS_FORWARD` sends what arrives on to other AEs, as a hospital routes a scanner's studies
to a viewer or a research archive, without anyone asking each time. A rule is a destination,
one of `PACS_DIMSE_MOVE_DESTINATIONS` by its id, and after a colon what a study must be to go
there:

```bash
PACS_DIMSE_REMOTES=viewer=VIEWER@10.0.0.5:104,research=RESEARCH@10.0.0.9:104
PACS_DIMSE_MOVE_DESTINATIONS=viewer,research
PACS_FORWARD='viewer,research:Modality=CT;StudyDescription=*LIVER*|*HEPAT*'
```

The first rule sends every study to the viewer, the second the CT studies of the liver to
the research archive. A key is matched as a search matches it (PS3.4 C.2.2.2): a value, or a
wild card with `*` and `?`, person names without regard to case; values joined by `|` are
alternatives, and every key of a rule must be met. The keys are those the index keeps:
PatientID, PatientName, AccessionNumber, StudyDescription, StudyID, ReferringPhysicianName;
Modality and SeriesDescription, met by one series of the study, the same one for both; and
Project, the [project](de-identification.md) the study came in through. A rule that cannot be read,
or whose destination is not one studies may be sent to, stops the start with the reason.

A study goes once it has been quiet for `PACS_FORWARD_AFTER` seconds (30), no instance of it
stored since, so that a scanner's study is sent whole rather than slice by slice. The index
notes it with the instance, in the same write, so a study stored is forwarded even when the
server stops before it is, and of several servers on one index only one sends it. Each
destination it meets gets a job of sending (`/jobs`, `Type` `C-STORE`),
taken up again after a restart like any other. A study that takes in instances later is
forwarded again, whole; the remote AE keeps what it already has.

## Being retrieved from

A viewer or a workstation has pacs-rs send it a study in one of two ways, on the Patient
Root or the Study Root model (PS3.4 C.6.1, C.6.2):

- **C-MOVE** (PS3.4 C.4.2) names where the study is to go; pacs-rs opens an association to
  that AE and sends each instance by C-STORE. The destination must be one pacs-rs is
  configured to send to, and reachable from it.
- **C-GET** (PS3.4 C.4.3) has the instances come back on the association that asks, so
  nothing has to be configured or opened towards the requester. The requester proposes a
  presentation context for each Storage SOP class it wants, with the SCP role for it
  (PS3.7 D.3.3.4); pacs-rs accepts the role for every Storage SOP class.

```bash
movescu -S -aet VIEWER -aec PACSRS -aem VIEWER +P 11113 -od received \
  -k QueryRetrieveLevel=STUDY -k StudyInstanceUID=1.2.840.113619.2.55.3.1 pacs.example 11112
getscu -S -aet VIEWER -aec PACSRS -od received \
  -k QueryRetrieveLevel=STUDY -k StudyInstanceUID=1.2.840.113619.2.55.3.1 pacs.example 11112
```

- **Who is answered, and what can be sent**: as for a [search](#being-searched). The AE
  titles of `PACS_DIMSE_REMOTES` are answered, another caller gets `0124`; under a
  project's AE title only that project's studies are there to send, and they are sent
  under that title.
- **What is asked for**: studies by a list of Study Instance UID; the series of one study;
  the instances of one series (PS3.4 C.6.2.1.5). On the Patient Root model each of these
  also names its patient by one Patient ID, and a retrieval at the PATIENT level sends
  every study of that patient (C.6.1.1.6); a study that is another patient's is not sent.
  An Identifier without those Unique Keys is refused (`A900`). What is asked for and is
  not there is not an error: Success, with nothing sent.
- **Responses**: a Pending one after each instance, with the numbers of Remaining,
  Completed, Failed and Warning sub-operations; then Success when every instance was
  stored, Warning (`B000`) when some were not or were stored with a warning, and a
  Failure (`A702`) when none was. Warning, Failure and Cancel come with the Failed SOP
  Instance UID List.
- **A C-GET sends an instance only on a context where the requester is the SCP**: an
  instance of a SOP class it proposed no context for, or took no SCP role for, is a failed
  sub-operation. With the role for no class at all, every one fails (`A702`).
- **A cancel** (C-CANCEL-MOVE-RQ, C-CANCEL-GET-RQ) is looked for between two instances,
  and for a C-GET while the requester has one, and ends with status Cancel: the instance
  on its way is finished, the rest is not sent.
- **The [audit trail](audit.md#what-is-recorded)** has what the destination stored, a line
  per study, with the AE that asked.

C-MOVE and C-GET statuses (PS3.4 Tables C.4-2 and C.4-3, PS3.7 9.1.4.1.7 and 9.1.3.1.6):

| Status | When |
|---|---|
| `FF00` Pending | an instance has been sent, or has failed; more remain |
| `0000` Success | every instance was stored by the destination; or there was none to send |
| `B000` Warning | some instances failed, or were stored with a warning |
| `FE00` Cancel | the move was cancelled |
| `A801` Refused: Move Destination unknown | C-MOVE: the Move Destination is not one of `PACS_DIMSE_MOVE_DESTINATIONS` |
| `A702` Refused: Out of resources, unable to perform sub-operations | no instance was stored: no association with the destination of a C-MOVE, no context with the SCP role for a C-GET, or every instance refused |
| `A701` Refused: Out of resources, unable to calculate number of matches | the index could not be read |
| `0124` Refused: Not authorized | the calling AE title is not one of `PACS_DIMSE_REMOTES` |
| `A900` Data Set does not match SOP Class | no Query/Retrieve Level, or one the model does not have; a Unique Key missing, empty or not a UID; several values above the level retrieved; on the Patient Root model, no single Patient ID; no Identifier |
| `C000` Unable to process | the Identifier cannot be read |

How the instances are sent, to the destination of a C-MOVE or the requester of a C-GET:

- **Destinations**: `PACS_DIMSE_MOVE_DESTINATIONS` names, by their `id`, the remote AEs of
  `PACS_DIMSE_REMOTES` that studies may be sent to. None unless named: the hospital's
  archive is among the remote AEs, and a research PACS only reads from it. A name that is
  not a remote AE, or two destinations with one AE title, stops the start.
- **Transfer syntax**: to a destination, pacs-rs proposes for each SOP class the transfer
  syntax the instance is stored in, and Explicit and Implicit VR Little Endian; on a C-GET
  the requester has proposed its own. A peer that takes the stored one
  gets the data set as stored, byte for byte; one that takes only uncompressed ones, as
  DCMTK's `storescp` does unless told otherwise, gets the pixel data decoded. What the
  codecs do not decode (colour, more than 16 bits) and the peer does not take as stored is
  not sent, and says so.
- **An instance whose SOP class the peer accepted no presentation context for is not sent**,
  and the others are: a failed sub-operation, as PS3.4 C.4.2.3.1 has it.
- A C-STORE that is the sub-operation of a move carries the Move Originator AE title and
  Message ID (PS3.7 9.3.1.1).

Tested with DCMTK 3.6.9 `movescu`, receiving itself and naming `storescp` as a third AE,
with `getscu`, as it comes and preferring JPEG-LS, and with pynetdicom 3.0.4 doing both,
on both models;
and the sending alone against `storescp`
(uncompressed only, and preferring JPEG-LS: the files it writes hold the original pixels
either way, checked with `dcmdjpls`) and a pynetdicom Storage SCP that takes Implicit VR
Little Endian only.

## Being searched

A viewer or a workstation searches pacs-rs with C-FIND on the Study Root or the Patient Root
Query/Retrieve Information Model (PS3.4 C.6.2, C.6.1), as it searches any archive:

```bash
findscu -S -aet VIEWER -aec PACSRS -k QueryRetrieveLevel=STUDY \
  -k "PatientName=DOE*" -k StudyDate=20260101-20260131 -k StudyInstanceUID pacs.example 11112
findscu -P -aet VIEWER -aec PACSRS -k QueryRetrieveLevel=PATIENT \
  -k "PatientName=DOE*" -k PatientID pacs.example 11112
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
  with its Study Instance UID; a series' instances, with both UIDs. On the Patient Root
  model it starts a level higher: patients; then a patient's studies, with its Patient ID,
  which every level below names too. Relational queries are not negotiated.
- **A patient** is the studies with its Patient ID that the caller sees: it is found by
  any of them, once, and described by one of them. A study without a Patient ID belongs to
  no patient, and is found on the Study Root model only.
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
| PATIENT (Patient Root) | Patient ID, Patient's Name; and Patient's Birth Date, Patient's Sex, Number of Patient Related Studies, Number of Patient Related Series, Number of Patient Related Instances |
| STUDY | Study Instance UID, Study Date, Study Time, Accession Number, Study ID; on the Study Root model Patient's Name and Patient ID too; and Modalities in Study, Referring Physician's Name, Study Description, Number of Study Related Series, Number of Study Related Instances; on the Study Root model Patient's Birth Date and Patient's Sex too |
| SERIES | Series Instance UID, Modality, Series Number; and Series Description, Number of Series Related Instances, Performed Procedure Step Start Date and Time, Request Attributes Sequence (Scheduled Procedure Step ID, Requested Procedure ID) |
| IMAGE | SOP Instance UID, Instance Number; and SOP Class UID, Number of Frames, Rows, Columns, Bits Allocated |

Each level has its Unique and Required Keys (PS3.4 Tables C.6-1 and C.6-2 for the Patient
Root model, C.6-5 for the Study Root one, C.6-3 and C.6-4 for both) and the Optional ones
listed after "and". What is the patient's is a key of the PATIENT level on the Patient Root
model, and of the STUDY level on the Study Root one, which has no PATIENT level. Any other key is left out of the responses, which then
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
| `A900` Data Set does not match SOP Class | no Query/Retrieve Level, or one the model does not have (PATIENT on the Study Root model); no single value in the Unique Key of a level above, the Patient ID included on the Patient Root model; no Identifier |
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

