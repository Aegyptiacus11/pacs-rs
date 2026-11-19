# DICOM: the network

How two DICOM systems send each other the files of [the previous page](dicom-files.md), in
the protocol that scanners and hospital archives speak: what the words mean, which of it
pacs-rs speaks today, and how to try it. The protocol's messages are
called DIMSE, and so is [the page that lists what pacs-rs supports](dimse.md).

## Who talks: application entities

A program that speaks DICOM on the network is an *application entity* (AE). To reach one
you need three things, and its administrator needs yours:

| | | For pacs-rs |
|---|---|---|
| AE title | its name: up to 16 characters (PS3.5 Section 6.2) | `PACS_DIMSE_AE_TITLE`, `PACSRS` unless set |
| Host | the machine's address | where pacs-rs runs |
| Port | the standard recommends 104, or 11112 where a program may not open ports below 1024 (PS3.8 Section 9.1.2) | `PACS_DIMSE_LISTEN`, e.g. `0.0.0.0:11112`; off until set |

**The AE title is a name, not a password.** It is sent in clear and anyone can claim one.
An archive can refuse callers it has not been told about, and so can pacs-rs
(`PACS_DIMSE_KNOWN_CALLERS_ONLY`), but that keeps out mistakes, not attackers: who may
connect is decided by the network, or by certificates: pacs-rs can speak this protocol
[over TLS](dimse.md#dimse-over-tls) and then takes only callers whose certificate a known
authority signed.

## Agreeing first: the association

Before any image moves, the caller opens an *association* and the two sides agree on what
they will do in it. The caller proposes a list of *presentation contexts*, each one a kind
of work (an *abstract syntax*, such as "store a CT image") with the transfer syntaxes it can
use for it; the other side accepts or refuses each one, and picks a single transfer syntax
for each it accepts (PS3.8 Section 7.1).

So a failure usually shows before anything is sent, and says which side must change:

| What the caller sees | Meaning |
|---|---|
| rejected: called AE title not recognised | it asked for a name this server does not have |
| rejected: calling AE title not recognised | this server only accepts callers it knows, and this is not one |
| no accepted presentation contexts | the server does not do the kind of work that was asked |

The side that asks for a service is its *user* (SCU) and the side that performs it its
*provider* (SCP). One program is often both: an archive is a provider when a scanner stores
to it, and a user when it sends a study on.

## The five messages

| Message | What it asks | pacs-rs as provider | pacs-rs as user |
|---|---|---|---|
| C-ECHO | "are you there?" (PS3.4 Annex A) | yes | yes: `POST /modalities/{id}/echo` |
| C-STORE | "keep this instance" (PS3.4 Annex B) | yes, into the archive | not yet |
| C-FIND | "which studies match?" (PS3.4 Annex C) | yes, to the AEs it knows: [being searched](dimse.md#being-searched) | yes: `GET /modalities/{id}/studies` |
| C-MOVE | "send these studies to that AE title" (PS3.4 Section C.4.2) | not yet | yes: `POST /modalities/{id}/move` |
| C-GET | "send these studies to me, on this association" (PS3.4 Section C.4.3) | not yet | not yet |

What is "not yet" is in the roadmap under [complete query and
retrieve](roadmap.md#4-complete-query-and-retrieve). In short: today a scanner or a
workstation can **send** to pacs-rs and **search** it, and pacs-rs can **search and pull
from** another archive. Another system cannot yet pull from pacs-rs with these messages; it
can over HTTP, with [DICOMweb](dicomweb.md).

### Why C-MOVE needs both sides configured

C-MOVE does not send images back to the one who asked. It names a destination by AE title
only, and the archive opens a new association to that destination and stores the images
there (PS3.4 Section C.4.2). The archive must therefore already know the host and port
behind that AE title. Until its administrator registers pacs-rs's AE title, host and
port, the archive answers *move destination unknown* and nothing is sent. pacs-rs says so in its answer when it happens ([DIMSE](dimse.md)).

C-GET avoids this by sending the images back on the association that asked
(PS3.4 Section C.4.3), which is why it is on the roadmap for archives that will not be
reconfigured.

## Try it

Start pacs-rs with the protocol on: in the quickstart's `compose.yaml`, add
`PACS_DIMSE_LISTEN: 0.0.0.0:11112` under `environment` and `"127.0.0.1:11112:11112"` under
`ports`. The commands below use [pynetdicom](https://pydicom.github.io/pynetdicom/)'s tools
through `uvx`, which needs no installation; DCMTK's `echoscu` and `storescu` name the called
AE title with `-aec` too.

Ask whether it is there. Without the right name, it refuses:

```bash
uvx --from pynetdicom python -m pynetdicom echoscu 127.0.0.1 11112 -v
```

```text
I: Requesting Association
E: Association Rejected
E: Result: Rejected Permanent, Source: Service User
E: Reason: Called AE title not recognised
```

With it (`-aec` is the called AE title):

```bash
uvx --from pynetdicom python -m pynetdicom echoscu 127.0.0.1 11112 -aec PACSRS -v
```

```text
I: Requesting Association
I: Association Accepted
I: Sending Echo Request: MsgID 1
I: Received Echo Response (Status: 0x0000 - Success)
I: Releasing Association
```

Send it the sample study, as a scanner would:

```bash
uvx --from pynetdicom python -m pynetdicom storescu 127.0.0.1 11112 . -aec PACSRS -v
```

```text
I: Association Accepted
I: Sending file: ./00001.dcm
I: Sending Store Request: MsgID 1, (CT)
I: Received Store Response (Status: 0x0000 - Success)
...
I: Releasing Association
```

The study is now in the archive, exactly as if it had been sent over HTTP: the same
identifiers, the same place in the web interface. Sent a second time it is answered Success
again and the first copy is kept.

Ask it a question it does not answer yet, and the refusal comes at the association:

```bash
uvx --from pynetdicom python -m pynetdicom findscu 127.0.0.1 11112 -aec PACSRS -S \
  -k QueryRetrieveLevel=STUDY -k PatientID=LITS-0
```

```text
I: Requesting Association
E: No accepted presentation contexts
```

## Connecting a hospital's archive

To pull studies from another archive, both sides are set up once:

1. On pacs-rs, name the archive in `PACS_DIMSE_REMOTES`, as `id=AE_TITLE@host:port`.
2. On the archive, its administrator registers pacs-rs: AE title, host and port.
3. Test the link with `POST /modalities/{id}/echo`.
4. Search with `GET /modalities/{id}/studies` and pull with `POST /modalities/{id}/move`.

The requests and their answers are in [DIMSE](dimse.md) and the
[REST API reference](rest-api.md). To have what is pulled de-identified on the way in, send
it to a project's AE title ([De-identification](de-identification.md)).

## The same work over HTTP

DICOMweb (PS3.18) does the same three things without associations or AE titles, which is
what web viewers and scripts use:

| DIMSE | DICOMweb | pacs-rs's REST API |
|---|---|---|
| C-STORE | STOW-RS: `POST /dicom-web/studies` | `POST /instances` |
| C-FIND | QIDO-RS: `GET /dicom-web/studies?...` | `POST /tools/find` |
| C-MOVE, C-GET | WADO-RS: `GET /dicom-web/studies/{uid}` | `GET /studies/{id}/archive` |

Over HTTP, pacs-rs has a [login](login.md) with roles, and an [audit trail](audit.md) covers
both ways in.
