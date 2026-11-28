# REST API reference

<!-- Written by `cargo run -p pacs-server --example api_reference`. Do not edit: change the handler's description, then run it again. -->

Every request of the REST API, written from the description each handler carries in the code: a request is on this page because the server serves it, and the answers are the ones its tests read. For how the requests fit together, see [REST API, task by task](rest-guide.md).

The same description is served by a running server at `/openapi.json` (OpenAPI 3.1), for a client generator or an API browser.

An identifier that names nothing is answered 404 with an empty body. Every other refusal is an [Error](#error); the login's own routes refuse with [Refused](#refused). With the [login](login.md) on, a request without a valid token, or with one that lacks the permission, is answered 401.

DICOMweb's own refusals are a [Problem](#problem).

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

### `GET /patients/{id}/labels`

The labels of a patient, sorted.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | list of string | Its labels. |
| 404 | nothing | No such resource; the body is empty. |

### `GET /patients/{id}/labels/{label}`

Whether a patient has a label: 200 if so, 404 if not; the body is empty.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `label` | path | string | The label. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It has the label; the body is empty. |
| 404 | nothing | No such resource, or it does not have the label; the body is empty. |

### `PUT /patients/{id}/labels/{label}`

Put a label on a patient: 1 to 64 ASCII letters, digits, `_` and `-`. Putting one it has changes nothing.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `label` | path | string | The label. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It has the label; the body is empty. |
| 400 | [Error](#error) | Not a label. |
| 404 | nothing | No such resource; the body is empty. |

### `DELETE /patients/{id}/labels/{label}`

Take a label off a patient. Taking one it does not have changes nothing.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `label` | path | string | The label. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It does not have the label; the body is empty. |
| 400 | [Error](#error) | Not a label. |
| 404 | nothing | No such resource; the body is empty. |

### `POST /patients/{id}/merge`

Merges a patient into another.

The patient's files take the other's Patient's Name, Patient ID, Birth Date and Sex, as
the other's latest study has them, and with them the other's identifier: their studies
are the other's from then on. As for a modification, the files replaced wait in the trash,
and restoring that entry undoes the merge.

Permission, when the login is on: `modify`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The identifier of the patient merged, who goes. |
| the body | | [Merge](#merge) | The patient kept. |

| Status | Answer | |
|---|---|---|
| 200 | [Modification](#modification) | Merged: the patient kept, and the trash entry that undoes it. |
| 400 | [Error](#error) | Not a merge, or a patient merged into themselves. |
| 403 | [Error](#error) | An account limited to projects. |
| 404 | nothing | Either patient is not there; the body is empty. |
| 409 | [Error](#error) | A study of the patient merged is a project's de-identified copy. |

### `GET /patients/{id}/metadata`

The metadata of a patient: their names, or names and values.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `expand` | query, optional | boolean | Names and values instead of names. `?expand` alone means true. |

| Status | Answer | |
|---|---|---|
| 200 | object | The names, sorted, or an object of names and values. |
| 404 | nothing | No such resource; the body is empty. |

### `GET /patients/{id}/metadata/{name}`

One metadata of a patient, as text.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `name` | path | string | The metadata's name. |

| Status | Answer | |
|---|---|---|
| 200 | `text/plain` | The value, as text. |
| 404 | nothing | No such resource, or no such metadata; the body is empty. |

### `PUT /patients/{id}/metadata/{name}`

Set a user's metadata of a patient: the body is the value.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `name` | path | string | The metadata's name. |
| the body | | `text/plain` | The value: UTF-8 text, at most 64 KiB. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | Set; the body is empty. |
| 400 | [Error](#error) | Not a name, or not a value. |
| 403 | [Error](#error) | A name of the server's own metadata. |
| 404 | nothing | No such resource; the body is empty. |

### `DELETE /patients/{id}/metadata/{name}`

Remove a user's metadata of a patient. Removing one it does not have changes nothing.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `name` | path | string | The metadata's name. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It has no such metadata now; the body is empty. |
| 400 | [Error](#error) | Not a name. |
| 403 | [Error](#error) | A name of the server's own metadata. |
| 404 | nothing | No such resource; the body is empty. |

### `POST /patients/{id}/modify`

Modifies a patient.

Patient's Name, Patient ID, Patient's Birth Date or Patient's Sex, each given, is written
into every stored file of the patient; the rest of each file, pixel data and UIDs included,
stays as it is. A new Patient ID gives the patient, their studies, series and instances new
identifiers, which the answer names: the old ones are gone, and labels and metadata move
to the new ones. The files replaced wait in the trash, and restoring that entry
(`POST /trash/{id}/restore`) undoes the change. A project's de-identified copy is not
modified.

Permission, when the login is on: `modify`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The patient's identifier. |
| the body | | [ModifyPatient](#modifypatient) | The new values; an attribute left out stays as it is. |

| Status | Answer | |
|---|---|---|
| 200 | [Modification](#modification) | Modified: the patient's identifier now, and the trash entry that undoes it. |
| 400 | [Error](#error) | No value given, or one that does not fit its value representation. |
| 403 | [Error](#error) | An account limited to projects. |
| 404 | nothing | No patient has this identifier; the body is empty. |
| 409 | [Error](#error) | A study of the patient is a project's de-identified copy. |

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

### `POST /studies/{id}/documents`

Adds a PDF document to a study.

The body is the PDF itself. It becomes an Encapsulated PDF instance (PS3.3 A.45.1) of the
study, in a series of its own (modality DOC, described by the title), with the study's
patient and study attributes; Content Date and Time are when it was added, in UTC. The
answer is that of `POST /instances`, and so are the refusals: an account limited to projects
adds nothing, nor does anyone to a study that came in through a project.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The study's identifier. |
| `title` | query, optional | string | The document's title, and its series' description (64 characters of it); "PDF document" if none. |
| the body | | `application/pdf` | The PDF. |

| Status | Answer | |
|---|---|---|
| 200 | [Stored](#stored) | Stored: the new instance. |
| 400 | [Error](#error) | The body is not a PDF. |
| 403 | [Error](#error) | The account is limited to some projects. |
| 404 | nothing | No study has this identifier; the body is empty. |
| 409 | [Error](#error) | The study came in through a project. |
| 413 | [Error](#error) | The PDF is larger than `PACS_MAX_UPLOAD_BYTES`. |

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

### `GET /studies/{id}/labels`

The labels of a study, sorted.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | list of string | Its labels. |
| 404 | nothing | No such resource; the body is empty. |

### `GET /studies/{id}/labels/{label}`

Whether a study has a label: 200 if so, 404 if not; the body is empty.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `label` | path | string | The label. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It has the label; the body is empty. |
| 404 | nothing | No such resource, or it does not have the label; the body is empty. |

### `PUT /studies/{id}/labels/{label}`

Put a label on a study: 1 to 64 ASCII letters, digits, `_` and `-`. Putting one it has changes nothing.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `label` | path | string | The label. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It has the label; the body is empty. |
| 400 | [Error](#error) | Not a label. |
| 404 | nothing | No such resource; the body is empty. |

### `DELETE /studies/{id}/labels/{label}`

Take a label off a study. Taking one it does not have changes nothing.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `label` | path | string | The label. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It does not have the label; the body is empty. |
| 400 | [Error](#error) | Not a label. |
| 404 | nothing | No such resource; the body is empty. |

### `GET /studies/{id}/metadata`

The metadata of a study: their names, or names and values.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `expand` | query, optional | boolean | Names and values instead of names. `?expand` alone means true. |

| Status | Answer | |
|---|---|---|
| 200 | object | The names, sorted, or an object of names and values. |
| 404 | nothing | No such resource; the body is empty. |

### `GET /studies/{id}/metadata/{name}`

One metadata of a study, as text.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `name` | path | string | The metadata's name. |

| Status | Answer | |
|---|---|---|
| 200 | `text/plain` | The value, as text. |
| 404 | nothing | No such resource, or no such metadata; the body is empty. |

### `PUT /studies/{id}/metadata/{name}`

Set a user's metadata of a study: the body is the value.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `name` | path | string | The metadata's name. |
| the body | | `text/plain` | The value: UTF-8 text, at most 64 KiB. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | Set; the body is empty. |
| 400 | [Error](#error) | Not a name, or not a value. |
| 403 | [Error](#error) | A name of the server's own metadata. |
| 404 | nothing | No such resource; the body is empty. |

### `DELETE /studies/{id}/metadata/{name}`

Remove a user's metadata of a study. Removing one it does not have changes nothing.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `name` | path | string | The metadata's name. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It has no such metadata now; the body is empty. |
| 400 | [Error](#error) | Not a name. |
| 403 | [Error](#error) | A name of the server's own metadata. |
| 404 | nothing | No such resource; the body is empty. |

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

### `GET /series/{id}/labels`

The labels of a series, sorted.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | list of string | Its labels. |
| 404 | nothing | No such resource; the body is empty. |

### `GET /series/{id}/labels/{label}`

Whether a series has a label: 200 if so, 404 if not; the body is empty.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `label` | path | string | The label. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It has the label; the body is empty. |
| 404 | nothing | No such resource, or it does not have the label; the body is empty. |

### `PUT /series/{id}/labels/{label}`

Put a label on a series: 1 to 64 ASCII letters, digits, `_` and `-`. Putting one it has changes nothing.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `label` | path | string | The label. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It has the label; the body is empty. |
| 400 | [Error](#error) | Not a label. |
| 404 | nothing | No such resource; the body is empty. |

### `DELETE /series/{id}/labels/{label}`

Take a label off a series. Taking one it does not have changes nothing.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `label` | path | string | The label. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It does not have the label; the body is empty. |
| 400 | [Error](#error) | Not a label. |
| 404 | nothing | No such resource; the body is empty. |

### `GET /series/{id}/metadata`

The metadata of a series: their names, or names and values.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `expand` | query, optional | boolean | Names and values instead of names. `?expand` alone means true. |

| Status | Answer | |
|---|---|---|
| 200 | object | The names, sorted, or an object of names and values. |
| 404 | nothing | No such resource; the body is empty. |

### `GET /series/{id}/metadata/{name}`

One metadata of a series, as text.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `name` | path | string | The metadata's name. |

| Status | Answer | |
|---|---|---|
| 200 | `text/plain` | The value, as text. |
| 404 | nothing | No such resource, or no such metadata; the body is empty. |

### `PUT /series/{id}/metadata/{name}`

Set a user's metadata of a series: the body is the value.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `name` | path | string | The metadata's name. |
| the body | | `text/plain` | The value: UTF-8 text, at most 64 KiB. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | Set; the body is empty. |
| 400 | [Error](#error) | Not a name, or not a value. |
| 403 | [Error](#error) | A name of the server's own metadata. |
| 404 | nothing | No such resource; the body is empty. |

### `DELETE /series/{id}/metadata/{name}`

Remove a user's metadata of a series. Removing one it does not have changes nothing.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `name` | path | string | The metadata's name. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It has no such metadata now; the body is empty. |
| 400 | [Error](#error) | Not a name. |
| 403 | [Error](#error) | A name of the server's own metadata. |
| 404 | nothing | No such resource; the body is empty. |

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

### `GET /instances/{id}/labels`

The labels of an instance, sorted.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | list of string | Its labels. |
| 404 | nothing | No such resource; the body is empty. |

### `GET /instances/{id}/labels/{label}`

Whether an instance has a label: 200 if so, 404 if not; the body is empty.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `label` | path | string | The label. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It has the label; the body is empty. |
| 404 | nothing | No such resource, or it does not have the label; the body is empty. |

### `PUT /instances/{id}/labels/{label}`

Put a label on an instance: 1 to 64 ASCII letters, digits, `_` and `-`. Putting one it has changes nothing.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `label` | path | string | The label. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It has the label; the body is empty. |
| 400 | [Error](#error) | Not a label. |
| 404 | nothing | No such resource; the body is empty. |

### `DELETE /instances/{id}/labels/{label}`

Take a label off an instance. Taking one it does not have changes nothing.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `label` | path | string | The label. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It does not have the label; the body is empty. |
| 400 | [Error](#error) | Not a label. |
| 404 | nothing | No such resource; the body is empty. |

### `GET /instances/{id}/metadata`

The metadata of an instance: their names, or names and values.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `expand` | query, optional | boolean | Names and values instead of names. `?expand` alone means true. |

| Status | Answer | |
|---|---|---|
| 200 | object | The names, sorted, or an object of names and values. |
| 404 | nothing | No such resource; the body is empty. |

### `GET /instances/{id}/metadata/{name}`

One metadata of an instance, as text.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `name` | path | string | The metadata's name. |

| Status | Answer | |
|---|---|---|
| 200 | `text/plain` | The value, as text. |
| 404 | nothing | No such resource, or no such metadata; the body is empty. |

### `PUT /instances/{id}/metadata/{name}`

Set a user's metadata of an instance: the body is the value.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `name` | path | string | The metadata's name. |
| the body | | `text/plain` | The value: UTF-8 text, at most 64 KiB. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | Set; the body is empty. |
| 400 | [Error](#error) | Not a name, or not a value. |
| 403 | [Error](#error) | A name of the server's own metadata. |
| 404 | nothing | No such resource; the body is empty. |

### `DELETE /instances/{id}/metadata/{name}`

Remove a user's metadata of an instance. Removing one it does not have changes nothing.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The resource's identifier. |
| `name` | path | string | The metadata's name. |

| Status | Answer | |
|---|---|---|
| 200 | nothing | It has no such metadata now; the body is empty. |
| 400 | [Error](#error) | Not a name. |
| 403 | [Error](#error) | A name of the server's own metadata. |
| 404 | nothing | No such resource; the body is empty. |

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

### `GET /instances/{id}/pdf`

An Encapsulated PDF instance's document.

The PDF as it was put in, without the padding DICOM adds to an odd length.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The instance's identifier. |

| Status | Answer | |
|---|---|---|
| 200 | `application/pdf` | The PDF. |
| 404 | nothing | No instance has this identifier, or it holds no PDF; the body is empty. |

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

A value matches as its attribute allows (PS3.4 C.2.2.2): itself; with `*` and `?` as
wild cards, for text and names; a range `from-to`, `from-` or `-to` for dates and
times; a list of UIDs separated by `\\`. Names match without regard to case, the rest
with. An empty value, or `*`, asks nothing.

- `Patient`: `PatientID`, `PatientName`, `PatientBirthDate`, `PatientSex`.
- `Study`: those, and `StudyInstanceUID`, `StudyDate`, `StudyTime`, `AccessionNumber`,
  `StudyID`, `StudyDescription`, `ReferringPhysicianName`, `ModalitiesInStudy`.
- `Series`: `SeriesInstanceUID`, needed. `Instance`: `SOPInstanceUID`, needed.

Another attribute, or a value that is not a match for its attribute, is refused.

`Labels` keeps the resources with all of them (`LabelsConstraint` `All`, the default),
any of them (`Any`) or none of them (`None`). With labels, a series or instance search
needs no UID, unless the constraint is `None`.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| the body | | [Find](#find) | The level and what to match. |

| Status | Answer | |
|---|---|---|
| 200 | [Found](#found) | The identifiers found, or the resources. |
| 400 | [Error](#error) | The body is not a search, names an attribute its level cannot search on, or gives a value that is not a match for its attribute. |

### `GET /tools/labels`

Every label in use on what the account sees, sorted.

Permission, when the login is on: `read`.

| Status | Answer | |
|---|---|---|
| 200 | list of string | The labels. |

## DICOMweb

The standard's own HTTP interface (PS3.18), under `/dicom-web` unless `PACS_DICOMWEB_PATH` says otherwise: what viewers speak. What of it is served, and how it matches and transcodes, is on [DICOMweb](dicomweb.md).

### `GET /dicom-web/studies`

Searches for studies (QIDO-RS).

Each study with its attributes, its counts of series and instances, the modalities of its series and the address to retrieve it from.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `{attribute}` | query, optional | string | A matching key, by keyword or tag, any number of them: see [Searching](dicomweb.md#searching) for the keys of each level and how a value matches. |
| `includefield` | query, optional | string | Attributes to add to each answer, by keyword or tag, comma-separated or repeated, or `all` for those of the level's modules: see [Searching](dicomweb.md#searching). |
| `limit` | query, optional | integer | At most this many. |
| `offset` | query, optional | integer | Skip this many first. |

| Status | Answer | |
|---|---|---|
| 200 | `application/dicom+json` | The studies, newest first. |
| 400 | [Problem](#problem) | A key this level does not have, one given twice, a value that is not a match for its key, or an `includefield` that names no attribute. |
| 406 | [Problem](#problem) | `Accept` asks for something else than this answers in. |

### `POST /dicom-web/studies`

Stores instances (STOW-RS).

Several files in one request. An instance already stored is kept as it was and counted as stored.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| the body | | `multipart/related` | One part an instance, each `application/dicom`: `multipart/related; type="application/dicom"`. Or `type="application/dicom+json"`: a first part of DICOM JSON, one data set an instance, and the parts its BulkDataURIs name by their Content-Location (the pixel data `application/octet-stream`, or compressed: `image/jls`, `image/jpeg`, `image/jp2`, `image/jxl`). |

| Status | Answer | |
|---|---|---|
| 200 | `application/dicom+json` | Every instance was stored. The answer names each one (PS3.18 Table 10.5.3-1). |
| 202 | `application/dicom+json` | Some were stored and some were not; the answer names both, each failure with its reason. |
| 400 | [Problem](#problem) | The body is not `multipart/related` with parts. |
| 403 | [Problem](#problem) | The account is limited to some projects: it stores nothing over HTTP. |
| 409 | `application/dicom+json` | None was stored; the answer names each failure with its reason. |
| 415 | [Problem](#problem) | The parts are neither `application/dicom` nor DICOM JSON or XML with bulk data. |

### `GET /dicom-web/studies/{study}`

Retrieves a study (WADO-RS).

Each instance as a part, `application/dicom`, in the transfer syntax it is stored in or the one `Accept` asks for ([compression and transcoding](dicomweb.md#compression-and-transcoding)).

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `study` | path | string | Study Instance UID. |

| Status | Answer | |
|---|---|---|
| 200 | `multipart/related` | The study's instances. |
| 404 | [Problem](#problem) | Nothing stored is named by this address. |
| 406 | [Problem](#problem) | `Accept` asks for something else than this answers in. |

### `POST /dicom-web/studies/{study}`

Stores instances into one study (STOW-RS).

As above, and an instance of another study is refused, the others being stored.

Permission, when the login is on: `write`.

| Send | Where | Type | |
|---|---|---|---|
| `study` | path | string | Study Instance UID. |
| the body | | `multipart/related` | One part an instance, each `application/dicom`: `multipart/related; type="application/dicom"`. Or `type="application/dicom+json"`: a first part of DICOM JSON, one data set an instance, and the parts its BulkDataURIs name by their Content-Location (the pixel data `application/octet-stream`, or compressed: `image/jls`, `image/jpeg`, `image/jp2`, `image/jxl`). |

| Status | Answer | |
|---|---|---|
| 200 | `application/dicom+json` | Every instance was stored. The answer names each one (PS3.18 Table 10.5.3-1). |
| 202 | `application/dicom+json` | Some were stored and some were not; the answer names both, each failure with its reason. |
| 400 | [Problem](#problem) | The body is not `multipart/related` with parts. |
| 403 | [Problem](#problem) | The account is limited to some projects: it stores nothing over HTTP. |
| 409 | `application/dicom+json` | None was stored; the answer names each failure with its reason. |
| 415 | [Problem](#problem) | The parts are neither `application/dicom` nor DICOM JSON or XML with bulk data. |

### `GET /dicom-web/studies/{study}/metadata`

A study's metadata (WADO-RS).

Every element but the pixel data, in DICOM JSON (PS3.18 Annex F).

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `study` | path | string | Study Instance UID. |

| Status | Answer | |
|---|---|---|
| 200 | `application/dicom+json` | One data set an instance. |
| 404 | [Problem](#problem) | Nothing stored is named by this address. |
| 406 | [Problem](#problem) | `Accept` asks for something else than this answers in. |

### `GET /dicom-web/studies/{study}/series`

Lists and searches a study's series (QIDO-RS).

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `study` | path | string | Study Instance UID. |
| `{attribute}` | query, optional | string | A matching key, by keyword or tag, any number of them: see [Searching](dicomweb.md#searching) for the keys of each level and how a value matches. |
| `includefield` | query, optional | string | Attributes to add to each answer, by keyword or tag, comma-separated or repeated, or `all` for those of the level's modules: see [Searching](dicomweb.md#searching). |
| `limit` | query, optional | integer | At most this many. |
| `offset` | query, optional | integer | Skip this many first. |

| Status | Answer | |
|---|---|---|
| 200 | `application/dicom+json` | The series, by Series Number; none for a study that is not there. |
| 400 | [Problem](#problem) | A key this level does not have, one given twice, a value that is not a match for its key, or an `includefield` that names no attribute. |
| 406 | [Problem](#problem) | `Accept` asks for something else than this answers in. |

### `GET /dicom-web/studies/{study}/series/{series}`

Retrieves a series (WADO-RS).

Each instance as a part, `application/dicom`, in the transfer syntax it is stored in or the one `Accept` asks for ([compression and transcoding](dicomweb.md#compression-and-transcoding)).

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `study` | path | string | Study Instance UID. |
| `series` | path | string | Series Instance UID. |

| Status | Answer | |
|---|---|---|
| 200 | `multipart/related` | The series' instances. |
| 404 | [Problem](#problem) | Nothing stored is named by this address. |
| 406 | [Problem](#problem) | `Accept` asks for something else than this answers in. |

### `GET /dicom-web/studies/{study}/series/{series}/instances`

Lists and searches a series' instances (QIDO-RS).

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `study` | path | string | Study Instance UID. |
| `series` | path | string | Series Instance UID. |
| `{attribute}` | query, optional | string | A matching key, by keyword or tag, any number of them: see [Searching](dicomweb.md#searching) for the keys of each level and how a value matches. |
| `includefield` | query, optional | string | Attributes to add to each answer, by keyword or tag, comma-separated or repeated, or `all` for those of the level's modules: see [Searching](dicomweb.md#searching). |
| `limit` | query, optional | integer | At most this many. |
| `offset` | query, optional | integer | Skip this many first. |

| Status | Answer | |
|---|---|---|
| 200 | `application/dicom+json` | The instances, by Instance Number; none for a series that is not there. |
| 400 | [Problem](#problem) | A key this level does not have, one given twice, a value that is not a match for its key, or an `includefield` that names no attribute. |
| 406 | [Problem](#problem) | `Accept` asks for something else than this answers in. |

### `GET /dicom-web/studies/{study}/series/{series}/instances/{instance}`

Retrieves an instance (WADO-RS).

Each instance as a part, `application/dicom`, in the transfer syntax it is stored in or the one `Accept` asks for ([compression and transcoding](dicomweb.md#compression-and-transcoding)).

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `study` | path | string | Study Instance UID. |
| `series` | path | string | Series Instance UID. |
| `instance` | path | string | SOP Instance UID. |

| Status | Answer | |
|---|---|---|
| 200 | `multipart/related` | The instance. |
| 404 | [Problem](#problem) | Nothing stored is named by this address. |
| 406 | [Problem](#problem) | `Accept` asks for something else than this answers in. |

### `GET /dicom-web/studies/{study}/series/{series}/instances/{instance}/bulkdata/{path}`

Retrieves bulk data of an instance (WADO-RS).

The value of one element that its metadata names by a BulkDataURI, as `application/octet-stream`.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `study` | path | string | Study Instance UID. |
| `series` | path | string | Series Instance UID. |
| `instance` | path | string | SOP Instance UID. |
| `path` | path | string | The element's tag, or tags and item numbers from 0 down to it, separated by `/`. |

| Status | Answer | |
|---|---|---|
| 200 | `multipart/related` | The value, one part. |
| 400 | [Problem](#problem) | Not a path of tags and item numbers. |
| 404 | [Problem](#problem) | No such instance, or no element of bytes there; the pixel data is retrieved by its frames. |
| 406 | [Problem](#problem) | `Accept` takes neither `application/octet-stream` nor a multipart of it. |

### `GET /dicom-web/studies/{study}/series/{series}/instances/{instance}/frames/{frames}`

Retrieves frames of an instance (WADO-RS).

Each frame as a part: uncompressed (`application/octet-stream`), or compressed as stored or as `Accept` asks.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `study` | path | string | Study Instance UID. |
| `series` | path | string | Series Instance UID. |
| `instance` | path | string | SOP Instance UID. |
| `frames` | path | string | Frame numbers from 1, separated by commas. |

| Status | Answer | |
|---|---|---|
| 200 | `multipart/related` | The frames, in the order asked. |
| 400 | [Problem](#problem) | Not frame numbers. |
| 404 | [Problem](#problem) | No such instance, no pixel data, or a frame past the last. |
| 406 | [Problem](#problem) | `Accept` asks for something else than this answers in. |
| 501 | [Problem](#problem) | The pixel data is in a form this server cannot give frames of. |

### `GET /dicom-web/studies/{study}/series/{series}/instances/{instance}/frames/{frame}/rendered`

Renders a frame of an instance (WADO-RS).

One frame, by its number from 1, as a picture: see the instance's.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `study` | path | string | Study Instance UID. |
| `series` | path | string | Series Instance UID. |
| `instance` | path | string | SOP Instance UID. |
| `frame` | path | string | One frame number, from 1. |
| `window` | query, optional | string | `center,width,function`. |
| `viewport` | query, optional | string | `vw,vh` or `vw,vh,sx,sy,sw,sh`. |
| `quality` | query, optional | integer | JPEG quality, 1 to 100 (90). |

| Status | Answer | |
|---|---|---|
| 200 | `image/jpeg` | The picture. |
| 400 | [Problem](#problem) | Not one frame number, or a parameter that is not valid. |
| 404 | [Problem](#problem) | No such instance, or a frame past the last. |
| 406 | [Problem](#problem) | `Accept` takes neither JPEG nor PNG. |
| 422 | [Problem](#problem) | An instance this server does not render, or a viewport region past the frame. |

### `GET /dicom-web/studies/{study}/series/{series}/instances/{instance}/metadata`

An instance's metadata (WADO-RS).

Every element but the pixel data, in DICOM JSON (PS3.18 Annex F).

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `study` | path | string | Study Instance UID. |
| `series` | path | string | Series Instance UID. |
| `instance` | path | string | SOP Instance UID. |

| Status | Answer | |
|---|---|---|
| 200 | `application/dicom+json` | The instance's data set. |
| 404 | [Problem](#problem) | Nothing stored is named by this address. |
| 406 | [Problem](#problem) | `Accept` asks for something else than this answers in. |

### `GET /dicom-web/studies/{study}/series/{series}/instances/{instance}/rendered`

Renders an instance (WADO-RS).

Its first frame as a picture: monochrome through its rescale and a window (the one asked for, else the file's first, else its range of values), colour as RGB.

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `study` | path | string | Study Instance UID. |
| `series` | path | string | Series Instance UID. |
| `instance` | path | string | SOP Instance UID. |
| `window` | query, optional | string | `center,width,function`, the function `linear`, `linear-exact` or `sigmoid`. |
| `viewport` | query, optional | string | `vw,vh`, or `vw,vh,sx,sy,sw,sh` with a region of the frame: the picture fits the viewport, its proportions kept. |
| `quality` | query, optional | integer | JPEG quality, 1 to 100 (90). |

| Status | Answer | |
|---|---|---|
| 200 | `image/jpeg` | The picture. |
| 400 | [Problem](#problem) | A parameter that is not one, or not a valid value. |
| 404 | [Problem](#problem) | No such instance. |
| 406 | [Problem](#problem) | `Accept` takes neither JPEG nor PNG. |
| 422 | [Problem](#problem) | An instance this server does not render (no pixel data, a photometric interpretation it does not take), or a viewport region past the frame. |

### `GET /dicom-web/studies/{study}/series/{series}/metadata`

A series' metadata (WADO-RS).

Every element but the pixel data, in DICOM JSON (PS3.18 Annex F).

Permission, when the login is on: `read`.

| Send | Where | Type | |
|---|---|---|---|
| `study` | path | string | Study Instance UID. |
| `series` | path | string | Series Instance UID. |

| Status | Answer | |
|---|---|---|
| 200 | `application/dicom+json` | One data set an instance, by Instance Number. |
| 404 | [Problem](#problem) | Nothing stored is named by this address. |
| 406 | [Problem](#problem) | `Accept` asks for something else than this answers in. |

## Remote archives

Other DICOM systems, reached over the DICOM network protocol: testing the link, searching their studies, bringing studies in.

### `GET /modalities`

Lists the remote AEs.

The AEs of `PACS_DIMSE_REMOTES`: those pacs-rs can test, search and pull studies from.

Permission, when the login is on: `remote`.

| Send | Where | Type | |
|---|---|---|---|
| `expand` | query, optional | boolean | Answer with each AE's title, host and port, by id, instead of the ids. |

| Status | Answer | |
|---|---|---|
| 200 | [RemoteAes](#remoteaes) | The ids, or the AEs by id. |

### `POST /modalities/{id}/echo`

Tests the link to a remote AE.

One C-ECHO from pacs-rs's AE title.

Permission, when the login is on: `remote`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The remote AE's id, as `PACS_DIMSE_REMOTES` names it. |

| Status | Answer | |
|---|---|---|
| 200 | [Echo](#echo) | The remote AE answered Success. |
| 404 | nothing | No remote AE has this id; the body is empty. |
| 502 | [Error](#error) | It could not be reached, refused the association, or answered otherwise; `Details` says which. |

### `POST /modalities/{id}/move`

Brings studies in from a remote AE.

A C-MOVE: the remote AE is asked to send the studies to pacs-rs, and they arrive through
pacs-rs's own C-STORE, as any instance sent to it. When the remote AE does not know
pacs-rs as a destination, takes no C-MOVE, or pacs-rs is not listening for DIMSE, a C-GET
instead, the studies coming back on the association that asks; `Method` says which was
made. Sent to a project, they are de-identified on the way in. A `Warning` header says
when the remote AE sent nothing, as it does for a study it does not have.

Permission, when the login is on: `remote`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The remote AE's id, as `PACS_DIMSE_REMOTES` names it. |
| the body | | [Move](#move) | The studies, and where they go. |

| Status | Answer | |
|---|---|---|
| 200 | [Moved](#moved) | The move is over. |
| 202 | [Started](#started) | With `Asynchronous`: the move started, as a job. |
| 400 | [Error](#error) | The body names no valid Study Instance UID, or a project that is not one. |
| 403 | [Error](#error) | The account is limited to some projects and `Project` is not one of them. |
| 404 | nothing | No remote AE has this id; the body is empty. |
| 502 | [Error](#error) | The remote AE could not be reached or failed, by C-MOVE and C-GET alike; `Details` says why. |

### `POST /modalities/{id}/store`

Sends studies to a remote AE.

A C-STORE of each instance of the studies, on one association pacs-rs opens. The remote AE
must be one instances may be sent to (`PACS_DIMSE_MOVE_DESTINATIONS`). The answer, once
all is sent, has the counts; with `Asynchronous` it is a job.

Permission, when the login is on: `remote`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The remote AE's id, as `PACS_DIMSE_REMOTES` names it. |
| the body | | [Send](#send) | The studies. |

| Status | Answer | |
|---|---|---|
| 200 | [Moved](#moved) | All is sent. |
| 202 | [Started](#started) | With `Asynchronous`: the sending started, as a job. |
| 400 | [Error](#error) | The body names no valid Study Instance UID. |
| 404 | [Error](#error) | No remote AE has this id (the body is empty), or a study is not in the archive, or not in the account's projects. |
| 409 | [Error](#error) | The remote AE is not one instances may be sent to. |
| 502 | [Error](#error) | The remote AE could not be reached, or stored none of the instances. |

### `GET /modalities/{id}/studies`

Searches a remote AE's studies.

A C-FIND at study level (Study Root), answered as a QIDO-RS search answers. The values
match as PS3.4 C.2.2.2 says: `*` and `?` in text and names, a date or a range of dates.
pacs-rs checks each match against the query as well, since an archive may not match on
every key. A `Warning` header says when more studies match than `limit` (the search is
then cancelled), when the archive did not match on every key, or when it ended early.

Permission, when the login is on: `remote`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The remote AE's id, as `PACS_DIMSE_REMOTES` names it. |
| `PatientID` | query, optional | string | Patient ID. |
| `PatientName` | query, optional | string | Patient's Name. |
| `AccessionNumber` | query, optional | string | Accession Number. |
| `StudyDate` | query, optional | string | A date, or a range: `20260101-20260131`, `-20260131`, `20260101-`. |
| `ModalitiesInStudy` | query, optional | string | One modality. |
| `limit` | query, optional | integer | At most this many studies: 100 unless given, 1000 at most. |

| Status | Answer | |
|---|---|---|
| 200 | `application/dicom+json` | The studies, in DICOM JSON. |
| 400 | [Error](#error) | A parameter that is not a search key here, or a value that breaks the matching rules. |
| 404 | nothing | No remote AE has this id; the body is empty. |
| 502 | [Error](#error) | The remote AE could not be reached or answered with a failure. |

## Jobs

Moves from a remote archive that run in the background.

### `GET /jobs`

Lists the jobs.

The moves running in the background and the latest 100 that are over, oldest first. They are kept in the index: a restart takes up the ones it was running. An account limited to projects has the jobs of its projects only.

Permission, when the login is on: `remote`.

| Status | Answer | |
|---|---|---|
| 200 | list of string | The jobs' ids. |

### `GET /jobs/{id}`

A job: its state, progress and counts.

Permission, when the login is on: `remote`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The job's id. |

| Status | Answer | |
|---|---|---|
| 200 | [Job](#job) | The job. |
| 404 | nothing | No job has this id, or none the account's projects have; the body is empty. |

### `POST /jobs/{id}/cancel`

Asks a running move to stop.

A C-CANCEL goes out with the remote AE's next response, and the job ends `Cancelled`
once that AE answers so (PS3.4 C.4.2.2.1): what it had sent by then stays.

Permission, when the login is on: `remote`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The job's id. |

| Status | Answer | |
|---|---|---|
| 202 | [Job](#job) | Asked to stop; the job as it is now. |
| 404 | nothing | No job has this id, or none the account's projects have; the body is empty. |
| 409 | [Error](#error) | The job is over already. |

## Trash

Deletions that can still be undone, for `PACS_TRASH_DAYS` days (30). For a role with `delete`, on an account not limited to projects.

### `GET /trash`

Lists the trash.

The deletions that can still be undone, the latest first. Empty when `PACS_TRASH_DAYS` is 0.

Permission, when the login is on: `delete`.

| Status | Answer | |
|---|---|---|
| 200 | list of [TrashEntry](#trashentry) | The deletions in the trash. |
| 403 | nothing | An account limited to projects. |

### `DELETE /trash/{id}`

Empties a deletion now.

Its files are removed for good; it cannot be restored after this.

Permission, when the login is on: `delete`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The entry's id. |

| Status | Answer | |
|---|---|---|
| 204 | nothing | Emptied. |
| 403 | nothing | An account limited to projects. |
| 404 | nothing | No such entry. |
| 503 | nothing | The store or the index did not answer. |

### `POST /trash/{id}/restore`

Restores a deletion.

Its files are indexed again as if received, each study in the project it was in, with the viewer's annotations of a whole study. An instance stored again since the deletion keeps the newer copy, and that file stays in the trash. The entry goes once no file is left in it.

Permission, when the login is on: `delete`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The entry's id. |

| Status | Answer | |
|---|---|---|
| 200 | [Restored](#restored) | What was restored, and what was left. |
| 403 | nothing | An account limited to projects. |
| 404 | nothing | No such entry. |
| 409 | nothing | A modification that cannot be undone whole: an instance deleted or modified again since. Nothing changed. |
| 503 | nothing | The store or the index did not answer. |

## Integrity

Each stored file checked against the size and SHA-256 the index took when it was stored. Only for a role with `users`.

### `GET /tools/check-files`

Where the checks of the stored files stand, and the files that failed.

Permission, when the login is on: `users`.

| Status | Answer | |
|---|---|---|
| 200 | [Status](#status) | The counts, the last whole-archive check, and every file whose last check failed. |

### `POST /tools/check-files`

Checks stored files against the index now.

With a `StudyInstanceUID`, that study's files are read and compared before the answer. Without one, the whole archive is checked in the background, one check at a time; `GET /tools/check-files` says how it went.

Permission, when the login is on: `users`.

| Send | Where | Type | |
|---|---|---|---|
| the body | | [CheckRequest](#checkrequest) | The study to check, or nothing for the whole archive. |

| Status | Answer | |
|---|---|---|
| 200 | [Summary](#summary) | The study was checked. |
| 202 | nothing | The whole archive is being checked. |
| 404 | nothing | No such study. |
| 409 | nothing | A whole-archive check is running already. |
| 503 | nothing | The store or the index did not answer: nothing was found wrong, nothing was checked to the end. |

## Signing in

With `PACS_AUTH=true`: sessions for people, and what a signed-in account may ask about itself.

### `POST /auth/download-token`

Makes a link a browser can follow.

A token good for one minute and one `GET` of one path, to append to it as `?token=`: for a download the browser starts itself, where no header can be set. Refused for a path the account may not read.

Permission, when the login is on: any signed-in user.

| Send | Where | Type | |
|---|---|---|---|
| the body | | [DownloadRequest](#downloadrequest) | The path the link is for. |

| Status | Answer | |
|---|---|---|
| 200 | [DownloadToken](#downloadtoken) | The token. |
| 403 | [Refused](#refused) | The account may not read this path, or the path is one no link is made for (the accounts). |
| 422 | [Refused](#refused) | Not a path of this server. |

### `POST /auth/login`

Signs in.

The access token comes in the answer, and the refresh token in an `HttpOnly` cookie
sent to `/auth/*` alone. A wrong password and an unknown login get the same answer.

Permission, when the login is on: none: it needs no token.

| Send | Where | Type | |
|---|---|---|---|
| the body | | [Credentials](#credentials) | The login and the password. |

| Status | Answer | |
|---|---|---|
| 200 | [Session](#session) | Signed in. |
| 401 | [Refused](#refused) | The login or the password is wrong, or the account is deactivated. |

### `POST /auth/logout`

Signs out of this session.

By the refresh cookie; the access token already given lasts until it runs out.

Permission, when the login is on: none: it needs no token.

| Status | Answer | |
|---|---|---|
| 204 | nothing | The session ended, if there was one. |

### `POST /auth/logout-all`

Signs out everywhere.

Ends every session of the user the access token is.

Permission, when the login is on: any signed-in user.

| Status | Answer | |
|---|---|---|
| 204 | nothing | Every session of the user ended. |

### `GET /auth/me`

Who the token is.

For a user or a tool, with the projects it is limited to.

Permission, when the login is on: any signed-in user.

| Status | Answer | |
|---|---|---|
| 200 | [Me](#me) | The account. |

### `POST /auth/password`

Changes the user's own password.

Every session of the user ends, this one included. Not for a tool, which has no password.

Permission, when the login is on: any signed-in user.

| Send | Where | Type | |
|---|---|---|---|
| the body | | [PasswordChange](#passwordchange) | The password now, and the new one: 12 to 1024 bytes. |

| Status | Answer | |
|---|---|---|
| 204 | nothing | Changed. |
| 403 | [Refused](#refused) | The current password is wrong. |
| 422 | [Refused](#refused) | The new password is too short or too long. |

### `POST /auth/refresh`

Carries a session on.

Spends the refresh cookie and sets its successor. A cookie already spent ends the
session, since two parties then hold it.

Permission, when the login is on: none: it needs no token.

| Status | Answer | |
|---|---|---|
| 200 | [Session](#session) | A new access token, and the next cookie. |
| 401 | [Refused](#refused) | No cookie, or one that is spent, expired or of a session that ended. |

## Users

With `PACS_AUTH=true`, for a role with `users`: who may sign in, with which role, on which projects.

### `GET /projects`

Lists the server's projects.

The names an account can be limited to: those of `PACS_PROJECTS`.

Permission, when the login is on: `users`.

| Status | Answer | |
|---|---|---|
| 200 | list of string | The projects' names. |

### `GET /users`

Lists the users.

Permission, when the login is on: `users`.

| Status | Answer | |
|---|---|---|
| 200 | list of [User](#user) | Every user, by login. |

### `POST /users`

Adds a user.

Permission, when the login is on: `users`.

| Send | Where | Type | |
|---|---|---|---|
| the body | | [NewUser](#newuser) | The login, the first password, the role, and the projects to limit them to. |

| Status | Answer | |
|---|---|---|
| 201 | [User](#user) | The user. |
| 409 | [Refused](#refused) | The login is taken. |
| 422 | [Refused](#refused) | A login with spaces, a password too short, a role or a project this server does not have. |

### `PUT /users/{id}`

Changes a user.

A new password or a deactivation ends the user's sessions at once. A new role reaches their access token at its next refresh, within 15 minutes. New projects hold from their next request.

Permission, when the login is on: `users`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The user's id. |
| the body | | [UserUpdate](#userupdate) | What to change; what is left out stays. |

| Status | Answer | |
|---|---|---|
| 200 | [User](#user) | The user after the change. |
| 404 | [Refused](#refused) | No such user. |
| 409 | [Refused](#refused) | The last active administrator cannot be deactivated or given another role. |
| 422 | [Refused](#refused) | A password too short, a role or a project this server does not have. |

### `DELETE /users/{id}`

Removes a user.

Their sessions end at once and the login is free again.

Permission, when the login is on: `users`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The user's id. |

| Status | Answer | |
|---|---|---|
| 204 | nothing | Removed. |
| 404 | [Refused](#refused) | No such user. |
| 409 | [Refused](#refused) | The last active administrator cannot be removed. |

## Roles

With `PACS_AUTH=true`: what each role allows. Read by anyone signed in, changed by a role with `users`.

### `GET /roles`

Lists the roles.

For any signed-in user, who may see what each role allows.

Permission, when the login is on: any signed-in user.

| Status | Answer | |
|---|---|---|
| 200 | list of [Role](#role) | Every role. |

### `POST /roles`

Adds a role.

Permission, when the login is on: `users`.

| Send | Where | Type | |
|---|---|---|---|
| the body | | [NewRole](#newrole) | Its name and its permissions. |

| Status | Answer | |
|---|---|---|
| 201 | [Role](#role) | The role. |
| 409 | [Refused](#refused) | A role has the id this name makes. |
| 422 | [Refused](#refused) | A name too long or without a letter or digit, or a permission that is not one. |

### `PUT /roles/{id}`

Changes a role.

Its name, its permissions, or both. New permissions reach each user's access token at its next refresh, and a tool at its next request.

Permission, when the login is on: `users`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The role's id. |
| the body | | [RoleUpdate](#roleupdate) | What to change; what is left out stays. |

| Status | Answer | |
|---|---|---|
| 200 | [Role](#role) | The role after the change. |
| 404 | [Refused](#refused) | No such role. |
| 409 | [Refused](#refused) | The `admin` role cannot be changed. |
| 422 | [Refused](#refused) | A name or a permission that is not valid. |

### `DELETE /roles/{id}`

Removes a role.

Permission, when the login is on: `users`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The role's id. |

| Status | Answer | |
|---|---|---|
| 204 | nothing | Removed. |
| 404 | [Refused](#refused) | No such role. |
| 409 | [Refused](#refused) | The `admin` role cannot be removed, nor a role a user or a tool still has. |

## Tools

With `PACS_AUTH=true`: programs that connect without a person, with an id, a secret and a token of five minutes.

### `POST /auth/token`

Gives a tool an access token.

The client credentials grant, as IHE's Internet User Authorization profile uses it (Get Access Token, ITI-71): the tool's id and secret go in `Authorization: Basic`, the rest in a form. The token is good for five minutes; the tool asks again when it has run out.

Permission, when the login is on: none: it needs no token.

| Send | Where | Type | |
|---|---|---|---|
| the body | | `application/x-www-form-urlencoded` | The grant, and optionally less than the role gives. |

| Status | Answer | |
|---|---|---|
| 200 | [Token](#token) | The token. |
| 400 | [TokenRefused](#tokenrefused) | The request is not a client credentials grant, or asks for a scope the role does not give or for another server. |
| 401 | [TokenRefused](#tokenrefused) | The id and secret are missing or not those of a tool. |

### `GET /clients`

Lists the tools.

Permission, when the login is on: `users`.

| Status | Answer | |
|---|---|---|
| 200 | list of [Tool](#tool) | Every tool, oldest first; never a secret. |

### `POST /clients`

Adds a tool.

The answer is the only place its secret ever appears: the index keeps a hash of it. With an OpenID provider, the tool is registered under its `client_id` there and has no secret here: it gets its tokens from the provider.

Permission, when the login is on: `users`.

| Send | Where | Type | |
|---|---|---|---|
| the body | | [NewClient](#newclient) | Its name, its role, and the projects to limit it to. |

| Status | Answer | |
|---|---|---|
| 201 | [Tool](#tool) | The tool, with its `secret`. |
| 422 | [Refused](#refused) | A name too long, a role or a project this server does not have. |

### `PUT /clients/{id}`

Changes a tool.

Its name, role or projects. They hold from its next request, tokens it already has included.

Permission, when the login is on: `users`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The tool's id. |
| the body | | [ClientUpdate](#clientupdate) | What to change; what is left out stays. |

| Status | Answer | |
|---|---|---|
| 200 | [Tool](#tool) | The tool after the change. |
| 404 | [Refused](#refused) | No such tool. |
| 422 | [Refused](#refused) | A name too long, a role or a project this server does not have. |

### `DELETE /clients/{id}`

Removes a tool.

Its secret gets no more tokens, and those it has stop at its next request.

Permission, when the login is on: `users`.

| Send | Where | Type | |
|---|---|---|---|
| `id` | path | string | The tool's id. |

| Status | Answer | |
|---|---|---|
| 204 | nothing | Removed. |
| 404 | [Refused](#refused) | No such tool. |

## Objects

What the requests above send and answer.

### Ancestor

The closest level still there after a deletion.

| Field | Type | |
|---|---|---|
| `ID` | string | Its identifier. |
| `Path` | string | Its address, such as `/series/{ID}`. |
| `Type` | string | `Patient`, `Study` or `Series`. |

### Attributes

A data set in DICOM JSON (PS3.18 Annex F): its attributes by tag, each with its `vr` and its `Value`.

object.

### CheckRequest

What to check.

| Field | Type | |
|---|---|---|
| `StudyInstanceUID` | string or null, optional | One study, checked before the answer. Absent: the whole archive, in the background. |

### ClientUpdate

What to change of a tool. What is left out stays as it is.

| Field | Type | |
|---|---|---|
| `name` | string or null, optional |  |
| `projects` | list of string or null, optional | Absent: no change. `null`: every study. A list: only those projects. |
| `role` | string or null, optional |  |

### CreateArchive

What to put in a ZIP.

| Field | Type | |
|---|---|---|
| `Resources` | list of string | Identifiers of patients, studies, series and instances, mixed as needed. |
| `Synchronous` | boolean, optional | `true` unless given; `false` is refused, as there are no archive jobs yet. |
| `Transcode` | string or null, optional | Refused when given: the files come as stored. |

### Credentials

A login and its password.

| Field | Type | |
|---|---|---|
| `email` | string | The login. The field keeps the name the sessions library gives it. |
| `password` | string |  |

### Deleted

The answer to a deletion.

| Field | Type | |
|---|---|---|
| `RemainingAncestor` | [Ancestor](#ancestor) or null | `null` when the deletion left the patient with nothing, so that it went too. |

### DicomJson

One study as a remote AE describes it: its attributes in DICOM JSON, by tag.

object.

### DownloadRequest

A link to make for a browser to follow.

| Field | Type | |
|---|---|---|
| `path` | string | A path of this server, with its query if any: `/studies/{id}/archive`. |

### DownloadToken

A token good for one `GET` of one path, to put in its URL as `?token=`.

| Field | Type | |
|---|---|---|
| `expires_in` | integer | Seconds it is good for. |
| `token` | string |  |

### Echo

A C-ECHO that the remote AE answered.

| Field | Type | |
|---|---|---|
| `AET` | string | The AE title that answered. |
| `ElapsedMs` | integer | From the first byte sent to the association's release. |
| `Status` | string | `Success`. |

### Error

Why a request was refused. An unknown identifier is a 404 with no body instead.

| Field | Type | |
|---|---|---|
| `Details` | string or null, optional | What exactly was wrong with this request. |
| `HttpError` | string | The status in words, such as `Bad Request`. |
| `HttpStatus` | integer | The status as a number. |
| `Message` | string | What kind of failure, such as `Bad file format` or `Not supported yet`. |

### FailedFile

A file whose last check failed.

| Field | Type | |
|---|---|---|
| `SOPInstanceUID` | string | The instance. |
| `SeriesInstanceUID` | string | Its series. |
| `StudyInstanceUID` | string | Its study. |
| `checked_ms` | integer | When it was found, in milliseconds since the Unix epoch. |
| `failure` | string | What was wrong. |

### Failure

A file that failed its check.

| Field | Type | |
|---|---|---|
| `SOPInstanceUID` | string | The instance whose file it is. |
| `failure` | string | What was wrong: changed (another SHA-256), a size other than the index's, or missing from the store. |

### Find

A search.

| Field | Type | |
|---|---|---|
| `Expand` | boolean, optional | Answer with the resources instead of their identifiers. |
| `Labels` | list of string, optional | Labels the resources must have, as `LabelsConstraint` says. |
| `LabelsConstraint` | string or null, optional | `All` (the default): every one of `Labels`; `Any`: at least one; `None`: none of them. |
| `Level` | string | `Patient`, `Study`, `Series` or `Instance`, in any case. |
| `Limit` | integer or null, optional | At most this many. |
| `Query` | object, optional | Attributes by keyword and the value each must equal: `{"PatientID": "LITS-0"}`. Empty or left out, a patient or study search finds them all; a series or instance search needs its UID. |
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
| `Labels` | list of string | Its labels, sorted (`/{level}/{id}/labels`). |
| `MainDicomTags` | object of string | SOP Instance UID, Instance Number and Image Position (Patient), where the file gives them. |
| `ParentSeries` | string | The identifier of the instance's series. |
| `Type` | string | `Instance`. |

### Job

A move in the background, or one that is over. Jobs are kept in the index, the latest hundred finished ones with them: a restart takes up the moves that were running.

| Field | Type | |
|---|---|---|
| `Completed` | integer or null |  |
| `ElapsedMs` | integer |  |
| `Error` | string or null | Why it failed; `null` unless `State` is `Failure`. |
| `Failed` | integer or null |  |
| `FailedSOPInstanceUIDs` | list of string |  |
| `ID` | string |  |
| `PatientID` | string or null | The Patient ID given with the move; `null` when none was. |
| `Progress` | integer or null | Percent of the instances done; `null` while the remote AE has given no count. |
| `Remaining` | integer or null | The counts of the latest response; `null` for one the remote AE has not given. |
| `Remote` | string | The id of the remote AE asked. |
| `Restarts` | integer | How many starts of the server took it up again, it having been running when the server stopped. |
| `State` | string | `Running`, `Success`, `Warning`, `Cancelled` or `Failure`. |
| `StudyInstanceUIDs` | list of string |  |
| `Type` | string | `C-MOVE`, `C-GET` once a move gave way to one, `C-STORE` for a send, or `WRITE-BACK` for a project's results sent back re-identified. |
| `Warning` | integer or null |  |

### Kept

A file a restore left in the trash.

| Field | Type | |
|---|---|---|
| `object_key` | string | Where it was before the deletion. |
| `reason` | string | Why it was left. |

### Me

Who a token is.

| Field | Type | |
|---|---|---|
| `id` | string |  |
| `login` | string | The user's login, or a tool's name. |
| `permissions` | list of string | What the role allows: of `read`, `write`, `remote`, `delete`, `users`. |
| `projects` | list of string or null | The projects the account is limited to; `null` when it sees every study. |
| `role` | string | The id of their role. |

### Merge

The patient another is merged into.

| Field | Type | |
|---|---|---|
| `Into` | string | The identifier of the patient kept. |

### Modification

What a modification or a merge did.

| Field | Type | |
|---|---|---|
| `ID` | string | The patient's identifier now. |
| `Instances` | integer | Instances whose files were written again. |
| `Path` | string |  |
| `Trash` | string or null | The trash entry holding the files replaced, whose restore undoes the change; `null` when the server keeps no trash (`PACS_TRASH_DAYS` 0). |

### ModifyPatient

New values for a patient's attributes. One left out stays as it is; an empty string removes a value, except the Patient ID's.

| Field | Type | |
|---|---|---|
| `PatientBirthDate` | string or null, optional | Patient's Birth Date, `YYYYMMDD`. |
| `PatientID` | string or null, optional |  |
| `PatientName` | string or null, optional | Patient's Name, as DICOM writes it: `FAMILY^GIVEN^MIDDLE^PREFIX^SUFFIX`. |
| `PatientSex` | string or null, optional | Patient's Sex: `M`, `F` or `O`. |

### Move

Which studies to bring in, and how.

| Field | Type | |
|---|---|---|
| `Asynchronous` | boolean or null, optional | Answer at once with a job, instead of when the move is over. |
| `PatientID` | string or null, optional | The Patient ID the studies belong to. A remote AE that retrieves in Patient Root only names a study under its patient, and is refused without it; one that takes Study Root does not need it. |
| `Project` | string or null, optional | The name of a project: the studies are sent to its AE title, and de-identified on the way in. Without it they come to pacs-rs's own AE title, as they are. |
| `StudyInstanceUID` | [Uids](#uids) |  |

### Moved

How a move ended.

| Field | Type | |
|---|---|---|
| `Completed` | integer or null | Instances sent, as the remote AE counted them; `null` when it gave no count. |
| `ElapsedMs` | integer |  |
| `Failed` | integer or null |  |
| `FailedSOPInstanceUIDs` | list of string | The instances the remote AE said it could not send. |
| `Method` | string | `C-MOVE`, `C-GET` when the move gave way to one, `C-STORE` for a send, or `WRITE-BACK` for a project's results sent back re-identified. |
| `Status` | string | `Success`, or `Warning` when some instances failed. |
| `Warning` | integer or null |  |

### NewClient

A tool to add.

| Field | Type | |
|---|---|---|
| `client_id` | string or null, optional | With an OpenID provider: the tool's client id there, which its tokens name. Without one it is not given: the server makes the id and a secret. |
| `name` | string |  |
| `projects` | list of string or null, optional | The projects to limit the client to; absent or `null`: every study. |
| `role` | string |  |

### NewRole

A role to add.

| Field | Type | |
|---|---|---|
| `name` | string |  |
| `permissions` | list of string, optional |  |

### NewUser

A user to add.

| Field | Type | |
|---|---|---|
| `login` | string |  |
| `password` | string |  |
| `projects` | list of string or null, optional | The projects to limit the user to; absent or `null`: every study. |
| `role` | string |  |

### PasswordChange

A user's own change of password.

| Field | Type | |
|---|---|---|
| `current_password` | string |  |
| `new_password` | string |  |

### Patient

A patient: the studies that share a Patient ID.

| Field | Type | |
|---|---|---|
| `ID` | string | The patient's identifier: the SHA-1 of the Patient ID. |
| `IsProtected` | boolean | Always `false`: pacs-rs never removes a patient to make room. |
| `IsStable` | boolean | Always `false` for now. |
| `Labels` | list of string | Its labels, sorted (`/{level}/{id}/labels`). |
| `LastUpdate` | string | When something under the patient last changed, as `YYYYMMDDTHHMMSS` in UTC. |
| `MainDicomTags` | object of string | Patient ID, Patient's Name, Birth Date and Sex, where the files give them. |
| `Studies` | list of string | The identifiers of the patient's studies. |
| `Type` | string | `Patient`. |

### PatientList

Identifiers, or with `expand` the patients themselves.

list of string or list of [Patient](#patient).

### Problem

A DICOMweb request refused: why.

| Field | Type | |
|---|---|---|
| `error` | string |  |

### Refused

A request refused by the login: why.

| Field | Type | |
|---|---|---|
| `detail` | string |  |

### RemoteAe

A remote AE as configured in `PACS_DIMSE_REMOTES`.

| Field | Type | |
|---|---|---|
| `AET` | string | Its AE title. |
| `Host` | string |  |
| `Port` | integer |  |

### RemoteAes

The remote AEs: their ids, or with `expand` each id's AE.

list of string or object of [RemoteAe](#remoteae).

### Restored

What a restore did.

| Field | Type | |
|---|---|---|
| `kept` | list of [Kept](#kept) | Files left in the trash, each with why: an instance stored again since the deletion keeps the newer copy. |
| `restored` | integer | Files indexed again. |

### Role

A role: a name for a set of permissions.

| Field | Type | |
|---|---|---|
| `id` | string | Made from the name: its letters and digits in lower case. |
| `locked` | boolean | `true` for the one role that cannot be changed or removed, `admin`. |
| `name` | string |  |
| `permissions` | list of string | Of `read`, `write`, `remote`, `delete`, `users`. |

### RoleUpdate

What to change of a role. What is left out stays as it is.

| Field | Type | |
|---|---|---|
| `name` | string or null, optional |  |
| `permissions` | list of string or null, optional |  |

### Send

Studies to send.

| Field | Type | |
|---|---|---|
| `Asynchronous` | boolean or null, optional | Answer at once with a job, instead of when all is sent. |
| `StudyInstanceUID` | [Uids](#uids) |  |

### Series

A series.

| Field | Type | |
|---|---|---|
| `ExpectedNumberOfInstances` | integer or null | Always `null`: pacs-rs does not work out how many instances a series should have. |
| `ID` | string | The series' identifier: the SHA-1 of `PatientID\|StudyInstanceUID\|SeriesInstanceUID`. |
| `Instances` | list of string | The identifiers of the series' instances, in Instance Number order. |
| `IsStable` | boolean | Always `false` for now. |
| `Labels` | list of string | Its labels, sorted (`/{level}/{id}/labels`). |
| `LastUpdate` | string | When something in the series last changed, as `YYYYMMDDTHHMMSS` in UTC. |
| `MainDicomTags` | object of string | Series Instance UID, Number and Description, and Modality, where the files give them. |
| `ParentStudy` | string | The identifier of the series' study. |
| `Status` | string | Always `Unknown`, for the reason of `ExpectedNumberOfInstances`. |
| `Type` | string | `Series`. |

### Session

A session begun or carried on: the access token to send as `Authorization: Bearer`. The refresh token comes beside it, in an `HttpOnly` cookie.

| Field | Type | |
|---|---|---|
| `access_token` | string | A JWT, good for 15 minutes. |
| `token_type` | string | `bearer`. |

### Started

A move started in the background.

| Field | Type | |
|---|---|---|
| `ID` | string | The job's id. |
| `Path` | string | Where to ask how it goes: `/jobs/{ID}`. |

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

### Status

Where the checks stand.

| Field | Type | |
|---|---|---|
| `failed` | integer | Files whose last check failed. |
| `failures` | list of [FailedFile](#failedfile) | Every file whose last check failed. |
| `files` | integer | Stored files. |
| `last_sweep` | [Sweep](#sweep) or null, optional | The last whole-archive check started on request, since the server started. |
| `never_checked` | integer | Stored before hashes were kept, and not checked since. |
| `running` | boolean | Whether a whole-archive check started on request is running. |

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
| `Labels` | list of string | Its labels, sorted (`/{level}/{id}/labels`). |
| `LastUpdate` | string | When something under the study last changed, as `YYYYMMDDTHHMMSS` in UTC. |
| `MainDicomTags` | object of string | Study Instance UID, Date, Time, ID and Description, Accession Number and Referring Physician's Name, where the files give them. |
| `ParentPatient` | string | The identifier of the study's patient. |
| `PatientMainDicomTags` | object of string | The patient's attributes, repeated here to save a request. |
| `Series` | list of string | The identifiers of the study's series. |
| `Type` | string | `Study`. |

### StudyList

Identifiers, or with `expand` the studies themselves.

list of string or list of [Study](#study).

### Summary

What a check of several files found.

| Field | Type | |
|---|---|---|
| `checked` | integer | Files checked. |
| `failures` | list of [Failure](#failure) | The files that failed, by SOP Instance UID, with what was wrong. |

### Sweep

The last whole-archive check started on request.

| Field | Type | |
|---|---|---|
| `checked` | integer | Files checked. |
| `error` | string or null, optional | Why it stopped before the end, when it did: the store or the index not answering. |
| `failed` | integer | Files that failed. |
| `finished_ms` | integer or null, optional | When it ended; `null` while it runs. |
| `started_ms` | integer | When it started, in milliseconds since the Unix epoch. |

### Token

An access token for a tool (OAuth 2.1, client credentials).

| Field | Type | |
|---|---|---|
| `access_token` | string | A JWT, to send as `Authorization: Bearer`. |
| `expires_in` | integer | Seconds it is good for. |
| `scope` | string | The permissions it carries, separated by spaces. |
| `token_type` | string | `Bearer`. |

### TokenRefused

A token request refused, as OAuth 2.1 words it.

| Field | Type | |
|---|---|---|
| `error` | string | `invalid_client`, `invalid_request`, `unsupported_grant_type`, `invalid_scope` or `invalid_target`. |
| `error_description` | string |  |

### TokenRequest

The form of a token request.

| Field | Type | |
|---|---|---|
| `grant_type` | string | `client_credentials`. |
| `requested_token_type` | string or null, optional | `urn:ietf:params:oauth:token-type:jwt`, the only kind. |
| `resource` | string or null, optional | This server's name (`PACS_AUTH_ISSUER`); another one is refused. |
| `scope` | string or null, optional | Permissions separated by spaces, to get fewer than the role gives. |

### Tool

A tool: a program that connects without a person.

| Field | Type | |
|---|---|---|
| `created_by` | string | The login of the user who made it. |
| `created_ms` | integer |  |
| `id` | string | What it presents, with its secret, to get a token. |
| `last_token_ms` | integer or null | When it last got a token; `null` when it never has. |
| `name` | string |  |
| `projects` | list of string or null | The projects the tool is limited to; `null` when it sees every study. |
| `role` | string | The id of its role. |
| `secret` | string or null, optional | Its secret: in the answer that makes the tool, and never again. |

### TrashEntry

A deletion in the trash.

| Field | Type | |
|---|---|---|
| `ID` | string | Its id, for restoring or emptying it. |
| `PublicID` | string | Its public ID. |
| `deleted_ms` | integer | When it was deleted, in milliseconds since the Unix epoch. |
| `emptied_ms` | integer | When the server empties it, in milliseconds since the Unix epoch. |
| `files` | integer | Files waiting. |
| `level` | string | What the deletion named: `Patient`, `Study`, `Series` or `Instance`; `Modification` for a patient modified or merged, whose restore undoes the change. |
| `studies` | list of object | The studies it took from, a part or all: `StudyInstanceUID`, `PatientID`, `PatientName`, and the `project` the study was in; for a modification, the values before it, and in `changed` the ones it gave. |

### Uids

One Study Instance UID, or several.

string or list of string.

### User

A user.

| Field | Type | |
|---|---|---|
| `active` | boolean | `false` for an account that may no longer sign in. |
| `created_ms` | integer | When the account was made, in milliseconds since 1970. |
| `id` | string |  |
| `login` | string | What they sign in with, in lower case. |
| `projects` | list of string or null | The projects the user is limited to; `null` when they see every study. |
| `role` | string | The id of their role. |

### UserUpdate

What to change of a user. What is left out stays as it is.

| Field | Type | |
|---|---|---|
| `active` | boolean or null, optional |  |
| `password` | string or null, optional |  |
| `projects` | list of string or null, optional | Absent: no change. `null`: every study. A list: only those projects. |
| `role` | string or null, optional |  |

