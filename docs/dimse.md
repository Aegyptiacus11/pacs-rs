# DIMSE services: the plan

Not built yet. The design follows `dicom-net` in
[segganeadel/dicom-rs-net](https://github.com/segganeadel/dicom-rs-net) (MIT or Apache-2.0),
read on 2026-10-31. pacs-rs writes its own on dicom-ul 0.10 instead of depending on it:
`dicom-net` builds against a sibling dicom-rs 0.9.1 checkout and a vendored dicom-ul patch that
is not upstream.

## Layers

```
pacs-server   AE configuration, wiring to archive, index and store
pacs-dimse    DIMSE messages, SCP routing, SCU (new crate)
dicom-ul      associations, PDUs, presentation contexts
TCP / TLS
```

## Taken from dicom-net

- One message type pairing the command set with its dataset; status codes; C-CANCEL.
- One handler per SOP class, found through a registry. The server supplies the hooks:
  C-STORE goes to `pacs-archive` ingest (the same path as a REST upload), C-FIND reads
  `pacs-index`, C-MOVE and C-GET read `pacs-store`.
- Several AE titles on one port, routed by called AE title. This needs the A-ASSOCIATE-RQ read
  before negotiation, which dicom-ul 0.10 does not offer: upstream a patch, or start with one AE
  per port.
- Move destinations from configuration (AE title to host and port).
- An association cap, timeouts, TLS; a live list of associations for the admin view, drained on
  shutdown.
- Large C-STORE datasets split across P-DATA PDUs.
- Interop tests against the dcm4che tools.

## Done differently

- C-FIND streams its matches from an index cursor, one pending response each, and checks
  C-CANCEL between them; `dicom-net` collects every match in memory first.
- Patient Root and Study Root from the start; `dicom-net` has Study Root only.
- C-MOVE and C-GET pending responses carry true remaining, completed, failed and warning counts.
- C-FIND needs the wildcard and range matching that `/tools/find` lacks today; both share it.

## Order

C-ECHO, C-STORE SCP, C-FIND SCP, C-MOVE SCP, C-GET SCP, then the SCU side.
