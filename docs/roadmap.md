# Roadmap

What pacs-rs does not do yet, gathered from the pages that describe each part. Nothing here is
ordered or dated, except the hospital interfaces, whose build order is in
[their plan](interop.md).

## Access

- A login. Today anyone who reaches the [REST API](rest-api.md) or the
  [web interface](web-interface.md) can delete, so the server belongs on a private network or
  behind an authenticating proxy.
- TLS on DIMSE associations, following PS3.15 H.1 (step 6 of the [plan](interop.md)).
- TLS on connections to Postgres ([Building and testing](building.md)).

## DICOMweb

From [DICOMweb](dicomweb.md):

- QIDO-RS wildcard, range, list and fuzzy matching (answered with 400 today, not ignored).
- `includefield` for attributes beyond the stored levels.
- Rendered media.
- STOW-RS with JSON or XML and bulk data.
- BulkDataURI in metadata.
- Lossless storage of colour images and of more than 16 bits, which are stored as received.

## REST API

From [REST API](rest-api.md):

- Labels, metadata and modifications.
- Wildcard and range searches in `/tools/find`.
- Deletions coordinated between two pacs-rs processes on one index.

## DIMSE

From [DIMSE](dimse.md) and the [plan](interop.md):

- Patient Root for C-FIND and C-MOVE (Study Root only today).
- C-GET SCU.
- C-STORE SCU, and with it the write-back of DICOM Segmentation and Structured Reports to a
  hospital's destination (clinical mode, step 7).
- The SCP side of query/retrieve, so that viewers and workstations can query pacs-rs (step 8).
- The Conformance Statement (PS3.2).
- Jobs that outlive a restart: moves in the background are kept in memory.

## De-identification

From [De-identification](de-identification.md):

- A re-identification key: the Encrypted Attributes Sequence of PS3.15 E.1.1, or an encrypted
  table.
- Retain Safe Private, Clean Structured Content, Clean Graphics and Clean Pixel Data.
- A review of the quarantine in the web interface.
- A run on the MIDI-B challenge's validation data; so far tested rule by rule against
  Table E.1-1.

## Not planned

Replacing a hospital's clinical PACS: HL7 v2 orders, Modality Performed Procedure Step, Storage
Commitment and the rest of IHE Scheduled Workflow.b
([why](interop.md#not-planned-replacing-a-clinical-pacs)).
