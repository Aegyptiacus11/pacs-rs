# DICOM: the network

The [previous page](dicom-files.md) opened a DICOM file. This one explains how two machines
send each other such files in the protocol that scanners and hospital archives speak, and
which parts of it pacs-rs speaks.

The messages of that protocol are called **DIMSE**. The page that lists everything pacs-rs
supports, status by status, has the same name: [DIMSE](dimse.md). To run every exchange
described here against your own server, do [Lab 3](lab-network.md).

## Who talks: application entities

A program that speaks DICOM on the network is an **application entity** (AE). To reach one
you need three things, and its administrator needs the same three about you:

| | What it is | For pacs-rs |
|---|---|---|
| AE title | its name: 16 characters at most (PS3.5 Section 6.2) | `PACS_DIMSE_AE_TITLE`, which is `PACSRS` unless you set it |
| Host | the address of the machine | where pacs-rs runs |
| Port | the standard recommends 104, or 11112 where a program may not open ports below 1024 (PS3.8 Section 9.1.2) | `PACS_DIMSE_LISTEN`, for example `0.0.0.0:11112`. The protocol is off until this is set |

> [!CAUTION]
> **An AE title is a name, not a password.** It is sent in clear, and anyone can claim any
> title. pacs-rs can refuse callers it has not been told about
> (`PACS_DIMSE_KNOWN_CALLERS_ONLY`), which keeps out mistakes and not attackers. Who may
> connect is decided by the network, or by certificates: pacs-rs can speak this protocol
> [over TLS](dimse.md#dimse-over-tls), and then takes only callers whose certificate was
> signed by an authority it knows.

## Agreeing first: the association

Before any image moves, the caller opens an **association**, and the two sides agree on what
they will do in it (PS3.8 Section 7.1):

1. The caller proposes a list of **presentation contexts**. Each one pairs a kind of work,
   called an *abstract syntax* (for example "store a CT image"), with the transfer syntaxes
   the caller can use for it.
2. The other side accepts or refuses each context, and picks one transfer syntax for each
   context it accepts.
3. Only then are messages exchanged, each in a context both sides agreed on.

Because of this, most failures show before a single image is sent, and the message says
which side has to change:

| What the caller sees | What it means |
|---|---|
| rejected: called AE title not recognised | the caller asked for a name this server does not have |
| rejected: calling AE title not recognised | this server accepts only callers it knows, and this is not one |
| no accepted presentation contexts | the server does not do the kind of work that was asked for |

The side that asks for a service is its **user** (SCU). The side that performs it is its
**provider** (SCP). One program is often both: an archive is a provider when a scanner stores
to it, and a user when it sends a study on to another archive.

## The five messages

| Message | What it asks | pacs-rs as provider | pacs-rs as user |
|---|---|---|---|
| C-ECHO | "are you there?" (PS3.4 Annex A) | yes | yes: `POST /modalities/{id}/echo` |
| C-STORE | "keep this instance" (PS3.4 Annex B) | yes, into the archive | yes, when it sends or forwards a study |
| C-FIND | "which studies match?" (PS3.4 Annex C) | yes, for the AE titles it knows | yes: `GET /modalities/{id}/studies` |
| C-MOVE | "send these studies to that AE title" (PS3.4 Section C.4.2) | yes, to the destinations it is given | yes: `POST /modalities/{id}/move` |
| C-GET | "send these studies to me, on this association" (PS3.4 Section C.4.3) | yes | not yet |

In plain words:

- A scanner or a workstation can **send** to pacs-rs, **search** it, and **pull from** it
  with C-MOVE or C-GET.
- pacs-rs can **search and pull from** another archive with C-FIND and C-MOVE.
- What is still missing is on the roadmap, under
  [complete query and retrieve](roadmap.md#4-complete-query-and-retrieve).

### Who may search and pull

Anyone who knows the AE title can echo and store. Searching and retrieving are answered only
for the AE titles listed in `PACS_DIMSE_REMOTES`. Another caller gets the status `0124`,
*Refused: Not authorized* ([being searched](dimse.md#being-searched)). The caution above still
holds: the list keeps out callers who were never set up, not someone who lies about their
title.

### Why C-MOVE needs both sides set up

C-MOVE does not send the images back to whoever asked. The request names a destination by its
AE title only, and the archive opens a *new* association to that destination and stores the
images there (PS3.4 Section C.4.2).

The archive must therefore already know the host and the port behind that AE title. Until its
administrator registers them, the archive answers *move destination unknown* (status `A801`)
and sends nothing. pacs-rs answers the same way for a destination it was not given
([being retrieved from](dimse.md#being-retrieved-from)).

C-GET avoids the second connection: the images come back on the association that asked
(PS3.4 Section C.4.3). It is the pull to use when the other side cannot be reconfigured.

## Connecting a hospital's archive

To pull studies from another archive, both sides are set up once:

1. On pacs-rs, name the archive in `PACS_DIMSE_REMOTES`, as `id=AE_TITLE@host:port`.
2. On the archive, its administrator registers pacs-rs: AE title, host and port.
3. Test the link with `POST /modalities/{id}/echo`.
4. Search with `GET /modalities/{id}/studies`, and pull with `POST /modalities/{id}/move`.

The requests and their answers are in [DIMSE](dimse.md) and the
[REST API reference](rest-api.md). To have what is pulled de-identified on its way in, send
it to a project's AE title ([De-identification](de-identification.md)).

## The same work over HTTP

DICOMweb (PS3.18) does the same three jobs with no association and no AE title. It is what
web viewers and scripts use.

| DIMSE | DICOMweb | pacs-rs's REST API |
|---|---|---|
| C-STORE | STOW-RS: `POST /dicom-web/studies` | `POST /instances` |
| C-FIND | QIDO-RS: `GET /dicom-web/studies?...` | `POST /tools/find` |
| C-MOVE, C-GET | WADO-RS: `GET /dicom-web/studies/{uid}` | `GET /studies/{id}/archive` |

Over HTTP, pacs-rs has a [login](login.md) with roles. The [audit trail](audit.md) records
both ways in.

## Where next

- Run an echo, a store, a search and a retrieval yourself:
  [Lab 3: DICOM on the network](lab-network.md).
- Every status and option of the protocol in pacs-rs: [DIMSE](dimse.md).
- Connect a viewer that speaks DICOMweb: [Connecting another viewer](viewers.md).
