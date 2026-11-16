# REST API reference

<!-- Written by `cargo run -p pacs-server --example api_reference`. Do not edit: change the handler's description, then run it again. -->

Every request of the REST API, written from the description each handler carries in the code: a request is on this page because the server serves it, and the answers are the ones its tests read. For how the requests fit together, see [REST API, task by task](rest-guide.md).

The same description is served by a running server at `/openapi.json` (OpenAPI 3.1), for a client generator or an API browser.

An identifier that names nothing is answered 404 with an empty body. Every other refusal is an [Error](#error). With the [login](login.md) on, a request without a valid token, or with one that lacks the permission, is answered 401.

Not on this page yet: the remote archives (`/modalities`, `/jobs`) and the login, users, roles and tools, which are on [REST API](rest-api.md) and [Login](login.md); and [DICOMweb](dicomweb.md).

## Patients

A patient is the studies that share a Patient ID.

### `GET /patients`

Lists the patients.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `expand` | query, optional | boolean | Answer with the resources instead of their identifiers. `?expand` alone means true. |
| `limit` | query, optional | integer | At most this many. |
| `since` | query, optional | integer | Skip this many first: with `limit`, a page. |

| Status | Answer | |
|---|---|---|
| 200 | [PatientList](#patientlist) | The patients' identifiers, or the patients. |
| 400 | [Error](#error) | `limit` or `since` is not a number. |

### `GET /patients/{id}`

A patient.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The patient's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Patient](#patient) | The patient. |
| 404 | nothing | No patient has this identifier; the body is empty. |

### `DELETE /patients/{id}`

Deletes a patient.

Everything under it goes too, and every level above it that is left empty. The index
entries are removed first and the files after. There is no undo.

Permission, when the login is on: `delete`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The patient's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Deleted](#deleted) | Deleted; the closest level still there. |
| 404 | nothing | No patient has this identifier; the body is empty. |

### `GET /patients/{id}/archive`

A patient's files in a ZIP, every study.

Folders by patient, study and series; the files as stored. Sent as it is built.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The patient's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | `application/zip` | The ZIP. |
| 404 | nothing | No patient has this identifier; the body is empty. |

### `GET /patients/{id}/instances`

A patient's instances, of every study.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The patient's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | list of [Instance](#instance) | The instances. |
| 404 | nothing | No patient has this identifier; the body is empty. |

### `GET /patients/{id}/series`

A patient's series, of every study.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The patient's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | list of [Series](#series) | The series. |
| 404 | nothing | No patient has this identifier; the body is empty. |

### `GET /patients/{id}/statistics`

How much a patient holds.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The patient's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Statistics](#statistics) | Counts and sizes. |
| 404 | nothing | No patient has this identifier; the body is empty. |

### `GET /patients/{id}/studies`

A patient's studies.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The patient's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | list of [Study](#study) | The studies. |
| 404 | nothing | No patient has this identifier; the body is empty. |

## Studies

One examination of a patient.

### `GET /studies`

Lists the studies.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `expand` | query, optional | boolean | Answer with the resources instead of their identifiers. `?expand` alone means true. |
| `limit` | query, optional | integer | At most this many. |
| `since` | query, optional | integer | Skip this many first: with `limit`, a page. |

| Status | Answer | |
|---|---|---|
| 200 | [StudyList](#studylist) | The studies' identifiers, or the studies. |
| 400 | [Error](#error) | `limit` or `since` is not a number. |

### `GET /studies/{id}`

A study.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The study's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Study](#study) | The study. |
| 404 | nothing | No study has this identifier; the body is empty. |

### `DELETE /studies/{id}`

Deletes a study.

Everything under it goes too, and every level above it that is left empty. The index
entries are removed first and the files after. There is no undo.

Permission, when the login is on: `delete`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The study's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Deleted](#deleted) | Deleted; the closest level still there. |
| 404 | nothing | No study has this identifier; the body is empty. |

### `GET /studies/{id}/archive`

A study's files in a ZIP.

Folders by patient, study and series; the files as stored. Sent as it is built.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The study's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | `application/zip` | The ZIP. |
| 404 | nothing | No study has this identifier; the body is empty. |

### `GET /studies/{id}/instances`

A study's instances, of every series.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The study's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | list of [Instance](#instance) | The instances. |
| 404 | nothing | No study has this identifier; the body is empty. |

### `GET /studies/{id}/patient`

The patient of a study.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The study's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Patient](#patient) | The patient. |
| 404 | nothing | No study has this identifier; the body is empty. |

### `GET /studies/{id}/series`

A study's series, in Series Number order.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The study's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | list of [Series](#series) | The series. |
| 404 | nothing | No study has this identifier; the body is empty. |

### `GET /studies/{id}/statistics`

How much a study holds.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The study's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Statistics](#statistics) | Counts and sizes. |
| 404 | nothing | No study has this identifier; the body is empty. |

## Series

One acquisition within a study.

### `GET /series/{id}`

A series.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The series's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Series](#series) | The series. |
| 404 | nothing | No series has this identifier; the body is empty. |

### `DELETE /series/{id}`

Deletes a series.

Everything under it goes too, and every level above it that is left empty. The index
entries are removed first and the files after. There is no undo.

Permission, when the login is on: `delete`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The series's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Deleted](#deleted) | Deleted; the closest level still there. |
| 404 | nothing | No series has this identifier; the body is empty. |

### `GET /series/{id}/instances`

A series' instances, in Instance Number order.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The series's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | list of [Instance](#instance) | The instances. |
| 404 | nothing | No series has this identifier; the body is empty. |

### `GET /series/{id}/patient`

The patient of a series.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The series's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Patient](#patient) | The patient. |
| 404 | nothing | No series has this identifier; the body is empty. |

### `GET /series/{id}/study`

The study of a series.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The series's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Study](#study) | The study. |
| 404 | nothing | No series has this identifier; the body is empty. |

## Instances

One stored DICOM file. Uploads arrive here, one file a request.

### `POST /instances`

Stores one DICOM file.

The file is filed under the patient, study and series it names. An instance already stored
is kept as it was and answered `AlreadyStored`, unless the server runs with
`PACS_OVERWRITE_INSTANCES`.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| the body | | `application/dicom` | One DICOM file, with its preamble and File Meta Information. |

| Status | Answer | |
|---|---|---|
| 200 | [Stored](#stored) | Stored, or already there. |
| 400 | [Error](#error) | The body is not a DICOM file. |
| 403 | [Error](#error) | The account is limited to some projects: what it sent would be in no project, so it stores nothing over HTTP. |
| 409 | [Error](#error) | The file names a series or instance that is stored under another study or series, or a study that came in through a project: only that project's AE title adds to it. |
| 413 | [Error](#error) | The file is larger than `PACS_MAX_UPLOAD_BYTES`, or would take too much memory to read. |

### `GET /instances/{id}`

An instance.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The instance's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Instance](#instance) | The instance. |
| 404 | nothing | No instance has this identifier; the body is empty. |

### `DELETE /instances/{id}`

Deletes an instance.

Everything under it goes too, and every level above it that is left empty. The index
entries are removed first and the files after. There is no undo.

Permission, when the login is on: `delete`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The instance's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Deleted](#deleted) | Deleted; the closest level still there. |
| 404 | nothing | No instance has this identifier; the body is empty. |

### `GET /instances/{id}/file`

An instance's DICOM file.

The file as stored: in JPEG-LS Lossless when pacs-rs compressed it on the way in. DICOMweb
retrieves it in another transfer syntax.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The instance's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | `application/dicom` | The DICOM file. |
| 404 | nothing | No instance has this identifier; the body is empty. |

### `GET /instances/{id}/patient`

The patient of an instance.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The instance's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Patient](#patient) | The patient. |
| 404 | nothing | No instance has this identifier; the body is empty. |

### `GET /instances/{id}/series`

The series of an instance.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The instance's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Series](#series) | The series. |
| 404 | nothing | No instance has this identifier; the body is empty. |

### `GET /instances/{id}/study`

The study of an instance.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The instance's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | [Study](#study) | The study. |
| 404 | nothing | No instance has this identifier; the body is empty. |

## Search and archives

Finding resources by their DICOM attributes, and several resources in one ZIP.

### `GET /tools/create-archive`

Several resources in one ZIP, by a link.

The same as the `POST`, for a link a browser can follow.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `resources` | query | string | Identifiers of any level, separated by commas. Without any, the ZIP is empty. |

| Status | Answer | |
|---|---|---|
| 200 | `application/zip` | The ZIP. |
| 404 | nothing | One of the identifiers names nothing; the body is empty. |

### `POST /tools/create-archive`

Several resources in one ZIP.

Patients, studies, series and instances together, each file once however many of the
resources hold it.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| the body | | [CreateArchive](#createarchive) | The resources to include. |

| Status | Answer | |
|---|---|---|
| 200 | `application/zip` | The ZIP. |
| 400 | [Error](#error) | The body is not an archive request, or asks for what is not supported yet. |
| 404 | nothing | One of the identifiers names nothing; the body is empty. |

### `POST /tools/find`

Searches by DICOM attributes at one level.

Exact matching only, on `PatientID` for patients; `StudyInstanceUID`, `PatientID`,
`AccessionNumber` and `StudyDate` for studies; `SeriesInstanceUID` for series;
`SOPInstanceUID` for instances. Wildcards, ranges and other attributes are refused, not
ignored.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| the body | | [Find](#find) | The level and what to match. |

| Status | Answer | |
|---|---|---|
| 200 | [Found](#found) | The identifiers found, or the resources. |
| 400 | [Error](#error) | The body is not a search, or asks for a match that is not supported yet. |

## Objects

What the requests above send and answer.

### Ancestor

The closest level still there after a deletion.

| Field | Type | |
|---|---|---|
| `ID` | string | Its identifier. |
| `Path` | string | Its address, such as `/series/{ID}`. |
| `Type` | string | `Patient`, `Study` or `Series`. |

### CreateArchive

What to put in a ZIP.

| Field | Type | |
|---|---|---|
| `Resources` | list of string | Identifiers of patients, studies, series and instances, mixed as needed. |
| `Synchronous` | boolean, optional | `true` unless given; `false` is refused, as there are no archive jobs yet. |
| `Transcode` | string or null, optional | Refused when given: the files come as stored. |

### Deleted

The answer to a deletion.

| Field | Type | |
|---|---|---|
| `RemainingAncestor` | [Ancestor](#ancestor) or null | `null` when the deletion left the patient with nothing, so that it went too. |

### Error

Why a request was refused. An unknown identifier is a 404 with no body instead.

| Field | Type | |
|---|---|---|
| `Details` | string or null, optional | What exactly was wrong with this request. |
| `HttpError` | string | The status in words, such as `Bad Request`. |
| `HttpStatus` | integer | The status as a number. |
| `Message` | string | What kind of failure, such as `Bad file format` or `Not supported yet`. |

### Find

A search.

| Field | Type | |
|---|---|---|
| `Expand` | boolean, optional | Answer with the resources instead of their identifiers. |
| `Level` | string | `Patient`, `Study`, `Series` or `Instance`, in any case. |
| `Limit` | integer or null, optional | At most this many. |
| `Query` | object, optional | Attributes by keyword and the value each must equal: `{"PatientID": "SYN1"}`. Empty or left out, a patient or study search finds them all; a series or instance search needs its UID. |
| `Since` | integer or null, optional | Skip this many first. |

### Found

What a search found: identifiers, or with `Expand` the resources of the level searched.

list of string or list of [Patient](#patient) or list of [Study](#study) or list of [Series](#series) or list of [Instance](#instance).

### Instance

An instance: one stored DICOM file.

| Field | Type | |
|---|---|---|
| `FileSize` | integer | Bytes of the file as stored, which is smaller than the file sent when pacs-rs compressed it. |
| `ID` | string | The instance's identifier: the SHA-1 of `PatientID\|StudyInstanceUID\|SeriesInstanceUID\|SOPInstanceUID`. |
| `IndexInSeries` | integer or null, optional | The Instance Number; left out when the file has none. |
| `Labels` | list of string | Always empty: labels are not supported yet. |
| `MainDicomTags` | object of string | SOP Instance UID, Instance Number and Image Position (Patient), where the file gives them. |
| `ParentSeries` | string | The identifier of the instance's series. |
| `Type` | string | `Instance`. |

### Patient

A patient: the studies that share a Patient ID.

| Field | Type | |
|---|---|---|
| `ID` | string | The patient's identifier: the SHA-1 of the Patient ID. |
| `IsProtected` | boolean | Always `false`: pacs-rs never removes a patient to make room. |
| `IsStable` | boolean | Always `false` for now. |
| `Labels` | list of string | Always empty: labels are not supported yet. |
| `LastUpdate` | string | When something under the patient last changed, as `YYYYMMDDTHHMMSS` in UTC. |
| `MainDicomTags` | object of string | Patient ID, Patient's Name, Birth Date and Sex, where the files give them. |
| `Studies` | list of string | The identifiers of the patient's studies. |
| `Type` | string | `Patient`. |

### PatientList

Identifiers, or with `expand` the patients themselves.

list of string or list of [Patient](#patient).

### Series

A series.

| Field | Type | |
|---|---|---|
| `ExpectedNumberOfInstances` | integer or null | Always `null`: pacs-rs does not work out how many instances a series should have. |
| `ID` | string | The series' identifier: the SHA-1 of `PatientID\|StudyInstanceUID\|SeriesInstanceUID`. |
| `Instances` | list of string | The identifiers of the series' instances, in Instance Number order. |
| `IsStable` | boolean | Always `false` for now. |
| `Labels` | list of string | Always empty: labels are not supported yet. |
| `LastUpdate` | string | When something in the series last changed, as `YYYYMMDDTHHMMSS` in UTC. |
| `MainDicomTags` | object of string | Series Instance UID, Number and Description, and Modality, where the files give them. |
| `ParentStudy` | string | The identifier of the series' study. |
| `Status` | string | Always `Unknown`, for the reason of `ExpectedNumberOfInstances`. |
| `Type` | string | `Series`. |

### Statistics

How much a patient or a study holds. The sizes are of the files as stored; the four sizes are the same number, kept under the names the reference server gives them.

| Field | Type | |
|---|---|---|
| `CountInstances` | integer |  |
| `CountSeries` | integer |  |
| `CountStudies` | integer or null, optional | For a patient only. |
| `DicomDiskSize` | string | Bytes, as a string. |
| `DicomDiskSizeMB` | integer | Whole mebibytes. |
| `DicomUncompressedSize` | string |  |
| `DicomUncompressedSizeMB` | integer |  |
| `DiskSize` | string |  |
| `DiskSizeMB` | integer |  |
| `UncompressedSize` | string |  |
| `UncompressedSizeMB` | integer |  |

### Stored

What became of a file sent to `POST /instances`.

| Field | Type | |
|---|---|---|
| `ID` | string | The instance's identifier. |
| `ParentPatient` | string |  |
| `ParentSeries` | string |  |
| `ParentStudy` | string |  |
| `Path` | string | The instance's address: `/instances/{ID}`. |
| `Status` | string | `Success` when the file was stored, `AlreadyStored` when an instance with its SOP Instance UID was there already and was kept. |

### Study

A study.

| Field | Type | |
|---|---|---|
| `ID` | string | The study's identifier: the SHA-1 of `PatientID\|StudyInstanceUID`. |
| `IsStable` | boolean | Always `false` for now. |
| `Labels` | list of string | Always empty: labels are not supported yet. |
| `LastUpdate` | string | When something under the study last changed, as `YYYYMMDDTHHMMSS` in UTC. |
| `MainDicomTags` | object of string | Study Instance UID, Date, Time, ID and Description, Accession Number and Referring Physician's Name, where the files give them. |
| `ParentPatient` | string | The identifier of the study's patient. |
| `PatientMainDicomTags` | object of string | The patient's attributes, repeated here to save a request. |
| `Series` | list of string | The identifiers of the study's series. |
| `Type` | string | `Study`. |

### StudyList

Identifiers, or with `expand` the studies themselves.

list of string or list of [Study](#study).

