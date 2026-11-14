# REST API

At the server's root, compatible with the reference server's REST API (see [Comparing with a
reference server](building.md#comparing-with-a-reference-server)), so its clients work unchanged. Resources have public IDs: the SHA-1 of their
DICOM identifiers (`pacs_core::public_ids`), the same as the reference server gives them.

| Request | |
|---|---|
| `POST /instances` | one DICOM file as the body; `ID`, `ParentPatient`, `ParentStudy`, `ParentSeries`, `Path`, `Status` (`Success` or `AlreadyStored`) |
| `GET /patients[?expand][&limit=&since=]`, `/studies[...]` | patient or study IDs, or the objects |
| `GET /patients/{id}`, `/studies/{id}`, `/series/{id}`, `/instances/{id}` | the resource, with its `MainDicomTags` |
| `GET /patients/{id}/studies`, `/patients/{id}/series`, `/patients/{id}/instances`, `/studies/{id}/series`, `/studies/{id}/instances`, `/series/{id}/instances` | descendants, expanded |
| `GET /studies/{id}/patient`, `/series/{id}/study`, `/series/{id}/patient`, `/instances/{id}/series`, `/instances/{id}/study`, `/instances/{id}/patient` | the ancestor |
| `GET /patients/{id}/statistics`, `/studies/{id}/statistics` | counts and sizes |
| `GET /patients/{id}/archive`, `/studies/{id}/archive` | the files in a ZIP, by patient, study and series |
| `GET /instances/{id}/file` | the DICOM file as stored |
| `DELETE /patients/{id}`, `/studies/{id}`, `/series/{id}`, `/instances/{id}` | the resource, everything under it and every ancestor left empty; `RemainingAncestor` names the closest one still there, or is `null` |
| `GET /tools/create-archive?resources=a,b`, `POST /tools/create-archive` (`{"Resources": [...]}`) | patients, studies, series and instances in one ZIP, laid out as above, each file once; a series or instance keeps its number in the series |
| `POST /tools/find` | `Level` Patient, Study, Series or Instance, exact matching on the UIDs, PatientID, AccessionNumber, StudyDate |
| `GET /modalities[?expand]` | the remote AEs of `PACS_DIMSE_REMOTES`: their ids, or each one's `AET`, `Host` and `Port` |
| `POST /modalities/{id}/echo` | a C-ECHO to that AE: 200 with `Status` and `ElapsedMs` when it answers Success, 502 with the reason when not |
| `GET /modalities/{id}/studies?PatientID=&PatientName=&AccessionNumber=&StudyDate=&ModalitiesInStudy=&limit=` | that AE's studies, by C-FIND (see [DIMSE](dimse.md)), as a DICOM JSON array |
| `POST /modalities/{id}/move` (`{"StudyInstanceUID": "..."}` or a list) | that AE sends the studies to pacs-rs by C-MOVE, or with `"Project"` to a project's AE title to be de-identified; the final status and counts, or with `"Asynchronous": true` a job |
| `GET /jobs`, `GET /jobs/{id}`, `POST /jobs/{id}/cancel` | the moves running in the background: state, progress and counts as they come; cancel sends C-CANCEL |

A patient is the studies that share its Patient ID. A deletion removes the index entries first,
then the files, so the index never names a missing file. A file that cannot be removed is
logged and left in the store, unindexed. Uploads wait while a deletion runs, so a file sent
again during a deletion is not lost. This holds within one server process: two pacs-rs
processes on one index do not coordinate yet.

**There is no login yet:** anyone who can reach the REST API can delete. Keep it on a private
network (see [Running](running.md)).

An instance sent again is kept as first stored (`AlreadyStored`) unless `PACS_OVERWRITE_INSTANCES`
is set. Lists of a study's series and a series' instances come in Series and Instance Number
order.

Not yet: labels, metadata, modifications, wildcard and range searches in `/tools/find`.
