# Audit trail

Off by default. `PACS_AUDIT_FILE` names a file, and the server then keeps a record of who did
what, apart from its application log and in the format a hospital's audit repository reads.

```bash
PACS_AUDIT_FILE=/var/log/pacs-rs/audit.log pacs-server
```

- The file is appended to, one line per event, and made readable by its owner alone when the
  server creates it. Its folder must exist.
- A file that cannot be opened stops the start: a server asked to keep a trail does not run
  without one.
- A line is written before the answer to the request leaves, so the trail never lacks an
  event a client saw succeed. Should a line fail to be written while the server runs, the
  application log says so, with the kind of event.
- Nothing that opens a door is written: no password, no secret, no token.
- **The file holds patient IDs and names**, since an audit trail says whose data was touched.
  Keep it as you keep the archive itself.

## What is recorded

About accounts and the server itself:

| Event | When |
|---|---|
| Application Activity (start, stop) | the server starts, and stops; with its AE titles when DIMSE is on |
| User Authentication, Login | `POST /auth/login`, and a tool's `POST /auth/token`: accepted or refused, with the login or ID that was claimed; named, as on its later lines, once accepted |
| User Authentication, Logout | `POST /auth/logout` and `/auth/logout-all` |
| Security Alert, User Security Attributes Changed | a user or a tool is made, changed or removed; a password is changed |
| Security Alert, Security Roles Changed | a role is made, changed or removed |
| Security Alert, Access Control Decision | a request the caller's role, or a tool's scope, does not allow |

About the data, for requests that succeeded; each names the study and its patient:

| Event | When |
|---|---|
| DICOM Instances Transferred, to the server | instances received: `POST /instances`, STOW-RS. One line per study in the request, with the number of instances of each SOP class; Create when the archive gained any, Update when it only replaced copies, Read when it held them all already |
| DICOM Instances Transferred, to the caller | instances, frames, rendered images and bulk data retrieved (WADO-RS), `GET /instances/{id}/file` |
| DICOM Instances Accessed, read | what a study is: its metadata, its lists of series and instances (QIDO-RS under a study, the REST API's resources), its measurements |
| DICOM Instances Accessed, update | a study's measurements replaced |
| DICOM Instances Accessed, delete | a series or an instance deleted, the study staying |
| DICOM Study Deleted | a study deleted whole (by itself, with its patient, or by its last series or instance), with the instances it had |
| Export | a ZIP archive or a media file-set downloaded, with the address it went to |
| Query | a search: QIDO-RS from the root, the REST API's lists and `/tools/find`, the web interface's lists; with what was asked |

**A request outside the account's projects.** An account [limited to some
projects](login.md#rights-per-project) that asks for a study outside them is answered 404, as
if the study were not there, and the trail says what happened: DICOM Instances Accessed with
the action that was tried (read, update or delete), the outcome minor failure, the
description `outside the account's projects`, and the study and patient that were named.
This holds for viewing, retrieving, measurements, archives and deletions, over the REST API,
DICOMweb and download links. A 404 for something that is in no study of the archive leaves no
line, for any account.

About DIMSE and the remote AEs:

| Event | When |
|---|---|
| DICOM Instances Transferred, to the server | an association is over: one line per study stored through it by C-STORE, from the calling AE title to the called one, with the number of instances of each SOP class |
| Security Alert, Node Authentication | a connection that did not become an association: an AE title that is not recognized, the limit of associations reached. Once per address and reason in 15 minutes |
| Query, of this server | a C-FIND it answered: the calling AE title asks, the called one answers; with the Information Model as the SOP Class queried, the Identifier as it was sent and its transfer syntax (A.5.3.10). Written when the Identifier was refused too, as a failure |
| DICOM Instances Transferred, from this server | a C-MOVE or a C-GET it answered: one line per study of what was stored at the other end, from the called AE title to the Move Destination, with the calling AE title as the requestor and the number of instances of each SOP class; for a C-GET the calling AE title is the destination. A retrieval that sent nothing leaves none |
| Security Alert, Access Control Decision | a C-FIND, a C-MOVE or a C-GET from an AE title the server does not answer. Once per address, AE title and operation in 15 minutes |
| Query, of a remote AE | `GET /modalities/{id}/studies` (C-FIND): this server asks, the remote AE answers, the caller is the requestor. Written when the remote AE fails too, as a failure |
| Begin Transferring DICOM Instances | `POST /modalities/{id}/move` (C-MOVE): the remote AE is asked to send studies here, one line per study. Their arrival is the first event of this table |

A study that arrives at a project's AE title is de-identified before it is stored, and its
line names the study and the patient as the archive holds them: the pseudonyms. Who the
patient was is not written. A study being moved is named by its UID alone, its patient
`unknown`: whose it is, the remote AE knows, and this server only once it has arrived.

Each event names who (`login@<source>` for a person, its ID for a tool, its AE title for a
DICOM application, `unknown` when the login is off), from which address, when, and how it
ended.

Reading one study takes many requests: a CT is hundreds of frames. The same caller reading,
being sent or searching the same thing again within 15 minutes leaves no second line, as the
profile allows (a message is not required each time the activity occurs). What is stored,
deleted or exported is written every time.

## Not yet

- **Quarantined instances.** An instance a project's profile sets aside is logged, not
  audited.
- **C-ECHO**, and the jobs of `/jobs`, leave no line.
- **Who signed out by cookie.** `POST /auth/logout` ends a session by its refresh cookie,
  which does not tell this layer whose it was: the event says `unknown`. `/auth/logout-all`,
  which comes with an access token, names the user.
- **Callers behind a proxy.** The address is the one that connected: behind a reverse proxy,
  the proxy's. `X-Forwarded-For` is not read.

## The format

A line is a syslog message as IHE's Record Audit Event [ITI-20] defines it (ITI TF-2:
3.20.4.1.2): RFC 5424, facility 10 (security) with severity 5, or 4 for a failure, the MSGID
`IHE+RFC-3881`, no structured data, and as its message the audit message of DICOM's Audit
Trail Message Format Profile (PS3.15 A.5), in XML on one line, UTF-8, without a byte order
mark.

```text
<85>1 2026-11-15T21:33:03.275Z pacs.example pacs-rs 4242 IHE+RFC-3881 - <AuditMessage><EventIdentification EventActionCode="E" EventDateTime="2026-11-15T21:33:03.274Z" EventOutcomeIndicator="0"><EventID csd-code="110114" codeSystemName="DCM" originalText="User Authentication"/><EventTypeCode csd-code="110122" codeSystemName="DCM" originalText="Login"/></EventIdentification><ActiveParticipant UserID="ada@pacs-rs" UserName="ada" UserIsRequestor="true" NetworkAccessPointID="192.0.2.7" NetworkAccessPointTypeCode="2"/><ActiveParticipant UserID="pacs-server@pacs.example" UserIsRequestor="false"/><AuditSourceIdentification AuditSourceID="pacs-rs"><AuditSourceTypeCode csd-code="4"/></AuditSourceIdentification></AuditMessage>
```

- Each event follows its table in PS3.15 A.5.3 (Application Activity A.5.3.1, Export A.5.3.4,
  Begin Transferring DICOM Instances A.5.3.3, DICOM Instances Accessed A.5.3.6, DICOM Instances Transferred A.5.3.7, DICOM Study Deleted
  A.5.3.8, Query A.5.3.10, Security Alert A.5.3.11, User Authentication A.5.3.12), with the
  codes of PS3.16 CID 400 to 403.
- A message is about one patient: a request that touches the studies of several leaves a
  line for each.
- A study is named by its Study Instance UID; a patient without an ID or a name, as
  de-identified data may be, is written `unknown`, since the fields may not be empty.
- A search that is not a DICOM query is recorded in its own terms, as A.5.3.10 allows: the
  path asked, and as the query its parameters or its body.
- The integer codes the schema inherited from RFC 3881 are written as ITI TF-2: 3.20.7.1.3
  asks: the integer, `RFC-3881` as the coding scheme, and RFC 3881's description.
- A DICOM application is named by its AE title, with `AETITLES=<title>` as its alternative
  ID (A.5.2.2).
- A tool that came with its access token is named as ITI-72 (3.72.5.1) names a caller with a
  JWT: `audience<subject@issuer>`.
- `AuditSourceID` is `PACS_AUDIT_SOURCE_ID` (`pacs-rs` by default), and the source type is 4,
  an application server.
- Times are UTC, to the millisecond.

The messages are built by the `pacs-audit` crate, which has no network or storage and can be
used on its own. Its tests validate every kind of message against the schema of PS3.15
A.5.1.1 ([Building and testing](building.md#the-audit-schema)).

## Sending to a repository

The file can be sent on to an audit record repository as it is written, by syslog over TLS
(RFC 5425), a transport of IHE's Record Audit Event [ITI-20].

```bash
PACS_AUDIT_FILE=/audit/audit.log
PACS_AUDIT_REPOSITORY=arr.hospital.example            # or host:port; 6514 unless given
PACS_AUDIT_REPOSITORY_CA_FILE=/secrets/site-ca.crt    # the authorities that sign its certificate
PACS_AUDIT_REPOSITORY_CERT_FILE=/secrets/pacs.crt     # what pacs-rs shows it, if it asks
PACS_AUDIT_REPOSITORY_KEY_FILE=/secrets/pacs.key
```

- **The file is the queue.** What is sent is the audit file, line by line, each as one
  message framed by its length (RFC 5425, 4.3). A position kept beside it (`audit.log.sent`)
  says how far the repository has been given it.
- **A repository that cannot be reached loses nothing.** The lines wait in the file, across
  restarts, for as long as it takes, and go in order once it answers; it is tried again
  after 1 second, then 2, up to a minute. The application log says when it stops answering
  and when it is back. A line is sent once; only a connection that breaks in the middle of
  a write can make the lines of that write arrive a second time.
- **Who the repository is, is checked**: its certificate must chain to an authority of
  `PACS_AUDIT_REPOSITORY_CA_FILE` and name the host in `PACS_AUDIT_REPOSITORY` (RFC 5425,
  5.2). A repository that does not check out gets nothing. TLS 1.2 and 1.3.
- **When the server stops**, it sends what is left, with the line that says it stops, and
  closes the connection properly (4.4), within ten seconds. What did not go is in the file
  and goes at the next start.
- **When the file is rotated** (replaced by an empty or shorter one), the new one is sent
  from its start. Rotate it only once the position has reached its end, or the lines not
  yet sent go with the old file.
- A start whose settings do not add up stops with the reason: a repository without an
  audit file or without authorities, a certificate without its key, a file that cannot be
  read.

What this transport cannot promise, here or in any sender: TLS acknowledges bytes, not
messages, so a repository that stops between taking a message off the wire and keeping it
loses that message. Not done: authorizing the repository by its certificate's fingerprint
(RFC 5425, 5.1), and the cipher suite that RFC made mandatory in 2009
(TLS_RSA_WITH_AES_128_CBC_SHA), which BCP 195 has since ruled out; and syslog over UDP.

