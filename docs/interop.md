# Hospital interfaces: the plan

Not built yet. pacs-rs speaks DICOMweb and its REST API today. Next to Quorum it is a **research PACS**
beside a hospital's clinical PACS: it pulls studies from the hospital, de-identifies them when the project
asks for it, and serves Quorum. It does not replace the clinical PACS.

Sources, checked on 2026-10-31: DICOM 2026d (PS3.2, PS3.4, PS3.7, PS3.15) and the IHE Radiology Technical
Framework, Revision 23.0 (8 August 2025). Behaviour is tested against DCMTK, dcm4che and pynetdicom. Other
Rust code, such as [segganeadel/dicom-rs-net](https://github.com/segganeadel/dicom-rs-net), is an example
to read, never a source for behaviour.

## The role

```
hospital PACS --C-FIND / C-MOVE--> pacs-rs staging --de-identify--> pacs-rs archive --> Quorum
(read-only source)                 (identified, never served)       (what readers see)
```

- **Research mode** (the default): every study is de-identified on ingest; only the de-identified copy is
  stored and served.
- **Clinical mode**: identified studies inside the hospital; results may go back to the hospital PACS.

The hospital PACS stays read-only for pacs-rs, except for a clinical-mode write-back to a destination the
hospital names.

## Build order

1. **C-ECHO**, SCU and SCP (PS3.7 9.1.5): test the link both ways.
2. **C-STORE SCP** (PS3.4 Annex B; statuses B.2.3): receives what a C-MOVE sends, and what a workstation
   pushes. Ingest goes through `pacs-archive` like a REST upload, into staging in research mode.
3. **C-FIND SCU** (PS3.4 C.4.1.2): Study Root first, then Patient Root; by Patient ID, Accession Number,
   Study Date range and Modality; the pending responses fill a result list as they arrive; C-CANCEL when
   the user stops.
4. **C-MOVE SCU** (PS3.4 C.4.2.2) with pacs-rs's own AE title as the destination; progress from the
   pending responses' Remaining, Completed, Failed and Warning counts. **C-GET SCU** (PS3.4 C.4.3.2) where
   the hospital supports it, with SCP/SCU Role Selection for the storage SOP classes (PS3.7 D.3.3.4). When
   the hospital offers DICOMweb, QIDO-RS and WADO-RS are the same steps over HTTP.
5. **De-identification on ingest** (research mode), below.
6. **TLS** on associations, and the configuration of remote AEs (title, host, port) and pacs-rs's own,
   following PS3.15 H.1.
7. **Write-back** (clinical mode only): Quorum's masks as DICOM Segmentation and measurements as Structured
   Reports, sent by C-STORE SCU to the hospital's destination. Never in research mode: the de-identified
   UIDs no longer match the hospital's.
8. **Later**: the SCP side of query/retrieve, so viewers and workstations can query pacs-rs (rules below);
   then the Conformance Statement (PS3.2), kept current from the first DIMSE release.

Patient lists from the hospital's records system (HL7 v2 or FHIR Patient and ImagingStudy) are optional, for
a site that wants them pushed; a C-FIND by Accession Number or Patient ID covers the usual request.

## De-identification

- The Basic Application Level Confidentiality Profile (PS3.15 E.2), with the options a project's approval
  allows, chosen per project and recorded with it: Retain Longitudinal Temporal Information With Modified
  Dates (E.3.6, dates shifted per patient so intervals survive), Clean Descriptors (E.3.5), Retain Patient
  Characteristics (E.3.7, such as sex and age), Retain Device Identity (E.3.8, such as scanner model).
- UIDs replaced consistently, so series and studies stay linked; one pseudonym per patient per project.
- Patient Identity Removed (0012,0062) set to YES and the De-identification Method Code Sequence
  (0012,0064) filled with the profile and options used (E.1.1).
- Clean Pixel Data (E.3.1): instances that may carry burned-in text (Burned In Annotation YES, secondary
  captures, dose screens) are refused unless cleaned.
- The re-identification key (E.1.2) is either kept encrypted and readable by one honest-broker role, or not
  kept at all. The project's ethics approval decides which.
- Staging is deleted once a study is de-identified; the identified copy is never indexed or served.
- Tested on the MIDI-B challenge's validation data (Data in Support of the MIDI-B Challenge,
  doi:10.7937/cf2p-aw56), whose answer key scores every required action, and against the action codes of
  PS3.15 Table E.1-1.

## Rules for the SCP side (step 8)

- **C-FIND** (PS3.4 C.4.1.3.1): one Pending response per match, each with a single match's values; a
  C-CANCEL interrupts the matching and ends with status Cancel. Matches stream from an index cursor.
- **Matching** (PS3.4 C.2.2.2): single value, list of UID, universal, wildcard, range, sequence, empty value
  and multiple value matching. One matcher serves C-FIND, QIDO-RS and `/tools/find`.
- **Information models** (PS3.4 C.6): Patient Root and Study Root; Patient/Study Only is retired.
- **C-MOVE** (PS3.4 C.4.2.3.1): Pending responses carry the Remaining, Completed, Failed and Warning counts;
  the final one is Success, Warning or Failure.
- **C-GET** (PS3.4 C.4.3.3.1): the C-STORE sub-operations run on the same association; role selection as
  above.

## What dicom-ul 0.10 already gives

Async and TLS associations (features `async`, `async-tls`), client and server; a custom `AccessControl`;
the called AE title of an established association; SCP/SCU role selection; custom negotiation. Several AE
titles on one port work when they share one set of presentation contexts; separate capabilities per AE
would need the A-ASSOCIATE-RQ before negotiation, which dicom-ul 0.10 does not offer and nothing here needs.

## What a site has to do

- Register pacs-rs's AE title, host and port on its PACS as a C-MOVE destination, and open the firewall
  between them.
- Agree the mode and, for research mode, the de-identification options and whether a key is kept, in the
  project's ethics or data-protection approval.

## Not planned: replacing a clinical PACS

A hospital's own archive is IHE Scheduled Workflow.b's Image Manager/Image Archive (RAD TF-1: 34). That would
add HL7 v2 orders and patient updates over MLLP [RAD-4, -12, -13], Modality Performed Procedure Step
[RAD-6, -7] (PS3.4 F.7), Storage Commitment [RAD-10] (PS3.4 J.3), Instance Availability Notification
[RAD-49] (PS3.4 Annex R) and rejection notes for deletions (IOCM). None of it is needed beside Quorum.
