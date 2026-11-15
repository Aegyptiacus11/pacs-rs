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

## What is recorded

| Event | When |
|---|---|
| Application Activity (start, stop) | the server starts, and stops; with its AE titles when DIMSE is on |
| User Authentication, Login | `POST /auth/login`, and a tool's `POST /auth/token`: accepted or refused, with the login or ID that was claimed |
| User Authentication, Logout | `POST /auth/logout` and `/auth/logout-all` |
| Security Alert, User Security Attributes Changed | a user or a tool is made, changed or removed; a password is changed |
| Security Alert, Security Roles Changed | a role is made, changed or removed |
| Security Alert, Access Control Decision | a request the caller's role, or a tool's scope, does not allow |

Each event names who (`login@<source>` for a person, its ID for a tool), from which address,
when, and how it ended (success, or a failure with the HTTP status).

## Not yet

- **What is read, stored, exported and deleted.** These are the events that name a study and
  its patient (DICOM Instances Transferred, DICOM Instances Accessed, DICOM Study Deleted,
  Query, Export): they are the next step, and the reason this page is not the whole
  [roadmap item](roadmap.md#1-safe-to-expose) yet.
- **Sending to a repository.** The lines are what IHE's Record Audit Event sends, but pacs-rs
  only writes them to the file. A syslog forwarder can ship the file as it is; sending over
  TLS from pacs-rs itself is not built.
- **Who signed out by cookie.** `POST /auth/logout` ends a session by its refresh cookie,
  which does not tell this layer whose it was: the event says `unknown`. `/auth/logout-all`,
  which comes with an access token, names the user.
- **Callers behind a proxy.** The address is the one that connected: behind a reverse proxy,
  the proxy's. `X-Forwarded-For` is not read.
- **DIMSE.** Associations are not audited yet.

## The format

A line is a syslog message as IHE's Record Audit Event [ITI-20] defines it (ITI TF-2:
3.20.4.1.2): RFC 5424, facility 10 (security) with severity 5, or 4 for a failure, the MSGID
`IHE+RFC-3881`, no structured data, and as its message the audit message of DICOM's Audit
Trail Message Format Profile (PS3.15 A.5), in XML on one line, UTF-8, without a byte order
mark.

```text
<85>1 2026-11-15T21:33:03.275Z pacs.example pacs-rs 4242 IHE+RFC-3881 - <AuditMessage><EventIdentification EventActionCode="E" EventDateTime="2026-11-15T21:33:03.274Z" EventOutcomeIndicator="0"><EventID csd-code="110114" codeSystemName="DCM" originalText="User Authentication"/><EventTypeCode csd-code="110122" codeSystemName="DCM" originalText="Login"/></EventIdentification><ActiveParticipant UserID="ada@pacs-rs" UserIsRequestor="true" NetworkAccessPointID="192.0.2.7" NetworkAccessPointTypeCode="2"/><ActiveParticipant UserID="pacs-server@pacs.example" UserIsRequestor="false"/><AuditSourceIdentification AuditSourceID="pacs-rs"><AuditSourceTypeCode csd-code="4"/></AuditSourceIdentification></AuditMessage>
```

- Each event follows its table in PS3.15 A.5.3 (Application Activity A.5.3.1, Security Alert
  A.5.3.11, User Authentication A.5.3.12), with the codes of PS3.16 CID 400 to 403.
- The integer codes the schema inherited from RFC 3881 are written as ITI TF-2: 3.20.7.1.3
  asks: the integer, `RFC-3881` as the coding scheme, and RFC 3881's description.
- A tool that came with its access token is named as ITI-72 (3.72.5.1) names a caller with a
  JWT: `audience<subject@issuer>`.
- `AuditSourceID` is `PACS_AUDIT_SOURCE_ID` (`pacs-rs` by default), and the source type is 4,
  an application server.
- Times are UTC, to the millisecond.

The messages are built by the `pacs-audit` crate, which has no network or storage and can be
used on its own. Its tests validate every kind of message against the schema of PS3.15
A.5.1.1 ([Building and testing](building.md#the-audit-schema)).
