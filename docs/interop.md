# Hospital interfaces: the plan

Not built yet. pacs-rs speaks DICOMweb and its REST API today; this is what a PACS needs besides
them in a hospital, and the rule each part follows.

Sources, checked on 2026-10-31: DICOM 2026d (PS3.2, PS3.4, PS3.7, PS3.15) and the IHE Radiology
Technical Framework, Revision 23.0 (8 August 2025), with its supplements. Behaviour is tested
against DCMTK, dcm4che and pynetdicom. Other Rust code, such as
[segganeadel/dicom-rs-net](https://github.com/segganeadel/dicom-rs-net), is an example to read,
never a source for behaviour.

## What the standards require

IHE Scheduled Workflow.b (RAD TF-1: 34) gives the Image Manager/Image Archive these required
transactions:

| Transaction | How | In pacs-rs |
|---|---|---|
| Modality Images Stored [RAD-8], Creator Images Stored [RAD-18] | DICOM C-STORE SCP | STOW-RS and REST upload only |
| Query Images [RAD-14], Images Availability Query [RAD-11] | C-FIND SCP | QIDO-RS only |
| Retrieve Images [RAD-16] | C-MOVE SCP | WADO-RS only |
| Storage Commitment [RAD-10] | N-ACTION, N-EVENT-REPORT (PS3.4 J.3) | no |
| Modality and Creator Procedure Step [RAD-6, -7, -20, -21] | MPPS: N-CREATE, N-SET (PS3.4 F.7) | no |
| Procedure Scheduled [RAD-4], Procedure Update [RAD-13] | HL7 v2 orders over MLLP | no |
| Patient Update [RAD-12] | HL7 v2 ADT over MLLP (A08 update, A40 merge) | no |

Optional there: Instance Availability Notification [RAD-49] (PS3.4 Annex R), Performed Work
Status Update [RAD-42].

So **HL7 v2 is required**: it is how the archive learns the orders (accession numbers) and the
patient corrections and merges, which it must apply to what it already stores.

**FHIR is not required** by Scheduled Workflow.b. It appears in Trial Implementation supplements:
MADO builds its manifest on the FHIR R4 ImagingStudy resource, and Results Distribution and
Encounter-Based Imaging Workflow use FHIR in part. ImagingStudy is trial-use in FHIR R5. pacs-rs
offers a read-only ImagingStudy endpoint whose Endpoint points at its DICOMweb, and no more until
a site asks.

Deletion and rejection of stored instances follow the IOCM supplement (rejection notes), which
the current REST deletion does not yet.

## DIMSE rules

- **C-FIND** (PS3.4 C.4.1.3.1): one Pending response per match, each with a single match's
  values; a C-CANCEL interrupts the matching and ends with status Cancel. Matches stream from
  an index cursor, never collected first.
- **Matching** (PS3.4 C.2.2.2): single value, list of UID, universal, wildcard, range, sequence,
  empty value and multiple value matching. One matcher serves C-FIND, QIDO-RS and `/tools/find`.
- **Information models** (PS3.4 C.6): Patient Root and Study Root; Patient/Study Only is retired.
- **C-MOVE** (PS3.4 C.4.2.3.1): Pending responses during the C-STORE sub-operations carry the
  Remaining, Completed, Failed and Warning counts; the final one is Success, Warning or Failure.
- **C-GET** (PS3.4 C.4.3.3.1): the C-STORE sub-operations run on the same association; the
  storage SOP classes are negotiated with SCP/SCU Role Selection (PS3.7 D.3.3.4).
- **C-STORE** statuses and conformance: PS3.4 B.2.3 and B.4.
- **Configuration**: devices, network AEs and connections as in PS3.15 H.1.
- **Conformance Statement** (PS3.2) once the first DIMSE service ships.

## What dicom-ul 0.10 already gives

Async and TLS associations (features `async`, `async-tls`), a custom `AccessControl`, the called
AE title of an established association, SCP/SCU role selection, and custom negotiation. Several
AE titles on one port therefore work when they share one set of presentation contexts: accept
the configured titles, then route by called AE title. Separate capabilities per AE would need
the A-ASSOCIATE-RQ before negotiation, which dicom-ul 0.10 does not offer; nothing here needs it.

## Order

1. C-ECHO and C-STORE SCP, through `pacs-archive` ingest like a REST upload.
2. The shared matcher, then the C-FIND SCP.
3. C-MOVE SCP with configured destinations, then C-GET.
4. Storage Commitment.
5. HL7 v2 over MLLP: ADT updates and merges applied to the index and stored headers; orders
   giving accession numbers.
6. MPPS.
7. FHIR ImagingStudy, read-only.
8. The Conformance Statement, kept current from then on.

Each rule above gets a test, and each service an interop test against DCMTK, dcm4che and
pynetdicom.
