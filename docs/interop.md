# Hospital interfaces: the plan

Steps 1 to 4 are built (`crates/pacs-dimse`, see [DIMSE](dimse.md)), C-GET aside; the rest is
not yet. pacs-rs speaks DICOMweb, its REST API and DIMSE C-ECHO, C-STORE, C-FIND and C-MOVE today. It is
a **research PACS** beside a hospital's clinical PACS: it pulls studies from the hospital, de-identifies
them when the project asks for it, and serves them to the research tools (viewers, annotation platforms,
pipelines) over DICOMweb. It does not replace the clinical PACS.

Sources, checked on 2026-10-31: DICOM 2026d (PS3.2, PS3.4, PS3.7, PS3.15) and the IHE Radiology Technical
Framework, Revision 23.0 (8 August 2025). Behaviour is tested against DCMTK, dcm4che and pynetdicom.

## The role

```
hospital PACS --C-FIND / C-MOVE--> pacs-rs staging --de-identify--> pacs-rs archive --> research tools
(read-only source)                 (identified, never served)       (what readers see)
```

- **Research mode**: a project, an AE title with a de-identification profile; every study sent to it is
  de-identified on ingest, and only the de-identified copy is stored and served.
- **Clinical mode**: studies sent to pacs-rs's own AE title are stored as received, inside the hospital;
  results may go back to the hospital PACS. pacs-rs is a general PACS: this is what it does with no
  project configured.

The hospital PACS stays read-only for pacs-rs, except for a clinical-mode write-back to a destination the
hospital names.

## Build order

1. **C-ECHO**, SCU and SCP (PS3.7 9.1.5): test the link both ways. *Done.*
2. **C-STORE SCP** (PS3.4 Annex B; statuses B.2.3): receives what a C-MOVE sends, and what a workstation
   pushes. Ingest goes through `pacs-archive` like a REST upload, into staging in research mode. *Done,
   without staging: `pacs_dimse::Destination` is where step 5 adds it.*
3. **C-FIND SCU** (PS3.4 C.4.1.2): Study Root first, then Patient Root; by Patient ID, Accession Number,
   Study Date range and Modality; the pending responses fill a result list as they arrive; C-CANCEL when
   the user stops. *Done for Study Root (`GET /modalities/{id}/studies`); Patient
   Root not yet. The SCU hands each match over as it arrives; the REST route collects them.*
4. **C-MOVE SCU** (PS3.4 C.4.2.2) with pacs-rs's own AE title as the destination; progress from the
   pending responses' Remaining, Completed, Failed and Warning counts. **C-GET SCU** (PS3.4 C.4.3.2) where
   the hospital supports it, with SCP/SCU Role Selection for the storage SOP classes (PS3.7 D.3.3.4). When
   the hospital offers DICOMweb, QIDO-RS and WADO-RS are the same steps over HTTP. *C-MOVE done for Study
   Root (`POST /modalities/{id}/move`), synchronous or as a job with its progress (`/jobs/{id}`). C-GET
   not yet.*
5. **De-identification on ingest** (research mode), below. *Built: `pacs-deid` and projects
   (`PACS_PROJECTS`); what is not yet is said below.*
6. **TLS** on associations, following PS3.15 H.1. Remote AEs (title, host, port) and pacs-rs's own are
   configured already (`PACS_DIMSE_*`).
7. **Write-back** (clinical mode only): segmentations as DICOM Segmentation and measurements as Structured
   Reports, sent by C-STORE SCU to the hospital's destination. Never in research mode: the de-identified
   UIDs no longer match the hospital's.
8. **Later**: the SCP side of query/retrieve, so viewers and workstations can query pacs-rs (rules below);
   then the Conformance Statement (PS3.2), kept current from the first DIMSE release.

Patient lists from the hospital's records system (HL7 v2 or FHIR Patient and ImagingStudy) are optional, for
a site that wants them pushed; a C-FIND by Accession Number or Patient ID covers the usual request.

## De-identification

Built:

- The Basic Application Level Confidentiality Profile, PS3.15 Table E.1-1 (2026d) applied whole, in
  sequence items too, with the options a project chooses: Retain UIDs, Device Identity, Institution
  Identity, Patient Characteristics (Patient's Age added from the dates), Full Dates, Modified Dates (one
  shift per patient, unlisted dates included, times kept), Clean Descriptors. Every choice a deployment may
  want differently is in the profile: options, per-tag overrides, the date shift range, the pseudonym
  prefix, the quarantine rules.
- UIDs, pseudonyms and date shifts derived from a project secret with HMAC, so the same patient and the
  same UIDs map the same way on every pull, with no table kept.
- Patient Identity Removed YES, the De-identification Method Code Sequence (CID 7050) and a
  De-identification Method naming the profile and each override (E.1.1); new File Meta Information.
- Quarantine instead of Clean Pixel Data (E.3.1), Clean Structured Content and Clean Graphics: Secondary
  Capture, encapsulated documents, SR, presentation states, ultrasound, Burned In Annotation YES, Image Type
  SCREEN SAVE; kept apart in `PACS_QUARANTINE`, or refused.
- No staging on disk: the identified instance is held in memory while it is de-identified, never stored.

Not yet:

- The re-identification key. The standard's way is the Encrypted Attributes Sequence (E.1.1): the
  original values inside each instance, encrypted to an honest broker's RSA key, so pacs-rs never holds a
  key that reads them. A separate encrypted table is the alternative; neither is built.
- Retain Safe Private, Clean Structured Content, Clean Graphics, Clean Pixel Data (OCR and review).
- A review of the quarantine in the web interface.
- Tested on the MIDI-B challenge's validation data (Data in Support of the MIDI-B Challenge,
  doi:10.7937/cf2p-aw56), whose answer key scores every required action; so far tested rule by rule
  against Table E.1-1.

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
[RAD-49] (PS3.4 Annex R) and rejection notes for deletions (IOCM). None of it is needed by a research PACS.
