# REST API, task by task

How a script or a pipeline sends, finds, fetches and deletes studies. This page goes through
the tasks in order on one study; the [REST API reference](api-reference.md) gives every
request in full.

Every command after a `$` here is run against a real server, with the answer shown, each time
the documentation changes ([how](building.md#the-documentations-examples)). They assume the
server and the three sample files of the [quickstart](quickstart.md), on an empty server.
`"..."` stands for a value that differs from one run to the next.

## Send files

One DICOM file a request, as the body of `POST /instances`:

```console
$ curl --data-binary @00001.dcm http://127.0.0.1:8080/instances
```

```json
{
  "ID": "3040830e-df3d6ba4-4ae11444-4aa8466e-e749ba5d",
  "ParentPatient": "11e5f1cd-80b3a875-5b7068ee-30e57e66-ca92c84c",
  "ParentSeries": "cd952d29-4afeecc6-6399a89b-609f9514-4198538e",
  "ParentStudy": "da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9",
  "Path": "/instances/3040830e-df3d6ba4-4ae11444-4aa8466e-e749ba5d",
  "Status": "Success"
}
```

The answer names the instance and the series, study and patient it was filed under. Sending
the other two slices fills the series:

```console
$ curl --data-binary @00002.dcm http://127.0.0.1:8080/instances
$ curl --data-binary @00003.dcm http://127.0.0.1:8080/instances
```

**Sending a file twice is safe.** The first copy is kept and the answer says so, with the
same identifiers, so a script that was interrupted can simply start again:

```console
$ curl --data-binary @00001.dcm http://127.0.0.1:8080/instances
```

```json
{
  "ID": "3040830e-df3d6ba4-4ae11444-4aa8466e-e749ba5d",
  "ParentPatient": "11e5f1cd-80b3a875-5b7068ee-30e57e66-ca92c84c",
  "ParentSeries": "cd952d29-4afeecc6-6399a89b-609f9514-4198538e",
  "ParentStudy": "da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9",
  "Path": "/instances/3040830e-df3d6ba4-4ae11444-4aa8466e-e749ba5d",
  "Status": "AlreadyStored"
}
```

To replace instead of keep, start the server with `PACS_OVERWRITE_INSTANCES=true`
([settings](running.md)).

What is not a DICOM file is refused, and nothing is stored:

```console
$ curl -d 'not a DICOM file' http://127.0.0.1:8080/instances
```

```json
{
  "HttpError": "Bad Request",
  "HttpStatus": 400,
  "Message": "Bad file format",
  "Details": "..."
}
```

To send many files in one request, use DICOMweb's STOW-RS ([DICOMweb](dicomweb.md)).

## Identifiers

The API names a patient, a study, a series and an instance by an identifier of five groups
of eight hexadecimal digits. It is not random: it is the SHA-1 of the DICOM identifiers
down to that level, joined by `|`.

| Level | SHA-1 of | For the sample |
|---|---|---|
| Patient | Patient ID | `SYN1` |
| Study | Patient ID, Study Instance UID | `SYN1\|2.25.1100000` |
| Series | the same, then Series Instance UID | `SYN1\|2.25.1100000\|2.25.1200000` |
| Instance | the same, then SOP Instance UID | `SYN1\|2.25.1100000\|2.25.1200000\|2.25.1300001` |

So a script that knows the DICOM identifiers can compute the address without asking:

```bash
printf 'SYN1|2.25.1100000' | sha1sum
# da6c0de8a32029efd216100a7bdd48563ea9bed9
# so the study is /studies/da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9
```

Two consequences. The same study has the same identifier on every pacs-rs server. And a
patient is the studies that share a Patient ID: pacs-rs does not decide whether two IDs are
one person.

## List what is there

```console
$ curl http://127.0.0.1:8080/patients
```

```json
["11e5f1cd-80b3a875-5b7068ee-30e57e66-ca92c84c"]
```

```console
$ curl http://127.0.0.1:8080/studies
```

```json
["da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9"]
```

With `?expand` the list holds the resources themselves instead of their identifiers. On a
large archive, read it in pages with `limit` and `since` (how many, and after how many):

```console
$ curl 'http://127.0.0.1:8080/studies?expand&limit=50&since=0'
```

```json
[
  {
    "ID": "da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9",
    "IsStable": false,
    "Labels": [],
    "LastUpdate": "...",
    "MainDicomTags": {
      "StudyDate": "20260101",
      "StudyTime": "120000",
      "AccessionNumber": "ACC1",
      "ReferringPhysicianName": "REFERRING^PHYSICIAN",
      "StudyDescription": "Synthetic study",
      "StudyInstanceUID": "2.25.1100000",
      "StudyID": "S1"
    },
    "ParentPatient": "11e5f1cd-80b3a875-5b7068ee-30e57e66-ca92c84c",
    "PatientMainDicomTags": {
      "PatientName": "SYNTHETIC^SERIES",
      "PatientID": "SYN1",
      "PatientBirthDate": "19700101",
      "PatientSex": "O"
    },
    "Series": ["cd952d29-4afeecc6-6399a89b-609f9514-4198538e"],
    "Type": "Study"
  }
]
```

`MainDicomTags` are the attributes of that level that the index keeps; `PatientMainDicomTags`
repeats the patient's, to save a request. `Series` are the identifiers one level down.

## Go down and up

Each resource has an address of its own, `/patients/{id}`, `/studies/{id}`, `/series/{id}` and
`/instances/{id}`, and answers with the object shown above. From one, the levels under it:

```console
$ curl http://127.0.0.1:8080/studies/da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9/series
```

```json
[
  {
    "ExpectedNumberOfInstances": null,
    "ID": "cd952d29-4afeecc6-6399a89b-609f9514-4198538e",
    "Instances": [
      "3040830e-df3d6ba4-4ae11444-4aa8466e-e749ba5d",
      "75d96295-373fe1cb-fc1a3f93-632477fc-ac17415d",
      "fbb232a1-b9bb23aa-d6fa59e2-a3d394f3-1e7dadde"
    ],
    "IsStable": false,
    "Labels": [],
    "LastUpdate": "...",
    "MainDicomTags": {
      "Modality": "CT",
      "SeriesDescription": "Synthetic series 1",
      "SeriesInstanceUID": "2.25.1200000",
      "SeriesNumber": "1"
    },
    "ParentStudy": "da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9",
    "Status": "Unknown",
    "Type": "Series"
  }
]
```

The instances of a series come in the order of their Instance Number:

```console
$ curl http://127.0.0.1:8080/series/cd952d29-4afeecc6-6399a89b-609f9514-4198538e/instances
```

```json
[
  {
    "FileSize": 1480,
    "ID": "3040830e-df3d6ba4-4ae11444-4aa8466e-e749ba5d",
    "IndexInSeries": 1,
    "Labels": [],
    "MainDicomTags": {
      "SOPInstanceUID": "2.25.1300001",
      "InstanceNumber": "1",
      "ImagePositionPatient": "0\\0\\0"
    },
    "ParentSeries": "cd952d29-4afeecc6-6399a89b-609f9514-4198538e",
    "Type": "Instance"
  },
  {
    "FileSize": 1628,
    "ID": "75d96295-373fe1cb-fc1a3f93-632477fc-ac17415d",
    "IndexInSeries": 2,
    "Labels": [],
    "MainDicomTags": {
      "SOPInstanceUID": "2.25.1300002",
      "InstanceNumber": "2",
      "ImagePositionPatient": "0\\0\\2.5"
    },
    "ParentSeries": "cd952d29-4afeecc6-6399a89b-609f9514-4198538e",
    "Type": "Instance"
  },
  {
    "FileSize": 1626,
    "ID": "fbb232a1-b9bb23aa-d6fa59e2-a3d394f3-1e7dadde",
    "IndexInSeries": 3,
    "Labels": [],
    "MainDicomTags": {
      "SOPInstanceUID": "2.25.1300003",
      "InstanceNumber": "3",
      "ImagePositionPatient": "0\\0\\5"
    },
    "ParentSeries": "cd952d29-4afeecc6-6399a89b-609f9514-4198538e",
    "Type": "Instance"
  }
]
```

And from any resource, the ones above it, here the patient of a series:

```console
$ curl http://127.0.0.1:8080/series/cd952d29-4afeecc6-6399a89b-609f9514-4198538e/patient
```

```json
{
  "ID": "11e5f1cd-80b3a875-5b7068ee-30e57e66-ca92c84c",
  "IsProtected": false,
  "IsStable": false,
  "Labels": [],
  "LastUpdate": "...",
  "MainDicomTags": {
    "PatientName": "SYNTHETIC^SERIES",
    "PatientID": "SYN1",
    "PatientBirthDate": "19700101",
    "PatientSex": "O"
  },
  "Studies": ["da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9"],
  "Type": "Patient"
}
```

How much a study or a patient holds:

```console
$ curl http://127.0.0.1:8080/studies/da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9/statistics
```

```json
{
  "CountInstances": 3,
  "CountSeries": 1,
  "DicomDiskSize": "4734",
  "DicomDiskSizeMB": 0,
  "DicomUncompressedSize": "4734",
  "DicomUncompressedSizeMB": 0,
  "DiskSize": "4734",
  "DiskSizeMB": 0,
  "UncompressedSize": "4734",
  "UncompressedSizeMB": 0
}
```

**Why 1480 bytes, when the file sent had 8968?** pacs-rs stores uncompressed images in
JPEG-LS, which is lossless: every pixel value comes back as it was sent, and the server
checks that before it keeps the compressed copy. `FileSize` and the statistics count the
files as stored. `PACS_COMPRESSION=off` keeps them as received
([compression](dicomweb.md#compression-and-transcoding)).

## Fetch files

One instance, as the DICOM file the server keeps:

```console
$ curl -o slice.dcm http://127.0.0.1:8080/instances/3040830e-df3d6ba4-4ae11444-4aa8466e-e749ba5d/file
```

A whole study or patient as a ZIP, in folders by patient, study and series
(`SYN1 - SYNTHETIC^SERIES/ACC1 - Synthetic study/1 - CT - Synthetic series 1/CT000001.dcm`).
It is sent file by file as it is built, so a large study does not wait in memory:

```console
$ curl -o study.zip http://127.0.0.1:8080/studies/da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9/archive
```

Several resources of any level in one ZIP, each file once:

```console
$ curl -o some.zip -d '{"Resources": ["3040830e-df3d6ba4-4ae11444-4aa8466e-e749ba5d", "cd952d29-4afeecc6-6399a89b-609f9514-4198538e"]}' \
    http://127.0.0.1:8080/tools/create-archive
```

The files come in the transfer syntax they are stored in. A program that needs another one,
or pixels without the file around them, asks through
[DICOMweb](dicomweb.md#compression-and-transcoding), which transcodes on request.

## Search

`POST /tools/find` takes a level and the attributes to match, and answers with identifiers:

```console
$ curl -d '{"Level": "Study", "Query": {"PatientID": "SYN1", "StudyDate": "20260101"}}' \
    http://127.0.0.1:8080/tools/find
```

```json
["da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9"]
```

`"Expand": true` answers with the resources, and `"Limit"` and `"Since"` page through them,
as in the lists above. What can be matched depends on the level:

| `Level` | Attributes |
|---|---|
| `Patient` | `PatientID` |
| `Study` | `StudyInstanceUID`, `PatientID`, `AccessionNumber`, `StudyDate` |
| `Series` | `SeriesInstanceUID` |
| `Instance` | `SOPInstanceUID` |

The match is exact. A search the server cannot do yet is refused with the reason, never
answered with something else:

```console
$ curl -d '{"Level": "Study", "Query": {"StudyDate": "20260101-20261231"}}' \
    http://127.0.0.1:8080/tools/find
```

```json
{
  "HttpError": "Bad Request",
  "HttpStatus": 400,
  "Message": "Not supported yet",
  "Details": "only exact matching is supported yet: StudyDate=20260101-20261231"
}
```

Ranges, wildcards and more attributes are on the
[roadmap](roadmap.md#4-complete-query-and-retrieve).

## Delete

Deleting a resource deletes everything under it, and every level above it that is left
empty. The answer names the closest level still there:

```console
$ curl -X DELETE http://127.0.0.1:8080/instances/3040830e-df3d6ba4-4ae11444-4aa8466e-e749ba5d
```

```json
{
  "RemainingAncestor": {
    "ID": "cd952d29-4afeecc6-6399a89b-609f9514-4198538e",
    "Path": "/series/cd952d29-4afeecc6-6399a89b-609f9514-4198538e",
    "Type": "Series"
  }
}
```

The series still has two instances, so it stays. Deleting the study takes the series, its
instances and the patient, who has no other study:

```console
$ curl -X DELETE http://127.0.0.1:8080/studies/da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9
```

```json
{"RemainingAncestor": null}
```

```console
$ curl http://127.0.0.1:8080/patients
```

```json
[]
```

**There is no undo.** The index entries go first and the files after, so the server never
lists a file it no longer has.

## With the login on

With `PACS_AUTH=true` every request above needs an access token, and what it may do depends
on the role behind it: reading, writing and deleting are separate permissions. A script is
a *tool*: an administrator gives it an ID and a secret once, and it exchanges them for a
five-minute token whenever it needs one ([Login: tools](login.md#tools)).

```bash
TOKEN=$(curl -s -u "$ID:$SECRET" -d grant_type=client_credentials \
  https://pacs.example/auth/token | jq -r .access_token)
curl -H "Authorization: Bearer $TOKEN" https://pacs.example/studies
```

## When a request fails

| Status | Meaning |
|---|---|
| 400 | the request is wrong or asks for what is not supported yet; the body says which, as above |
| 401 | the login is on and the token is missing, expired or lacks the permission |
| 404 | no resource has this identifier; the body is empty |
| 413 | the file is larger than `PACS_MAX_UPLOAD_BYTES`, or would take too much memory to read |
| 429, 503 | too many requests at once, from this address or for the server: wait the `Retry-After` seconds and send it again ([limits](running.md#limits-on-requests)) |

## The rest of the API

- Other archives over DIMSE: listing them, testing the link, searching their studies and
  pulling them in, with jobs for long moves. See [DIMSE](dimse.md) and the
  [reference](rest-api.md).
- Users, roles and tools: [Login](login.md).
