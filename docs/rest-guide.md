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
  "ID": "518d98a6-f5a3181d-2984303f-271f3d0d-c973dfc2",
  "ParentPatient": "64c94bda-110c6fa1-720a3d46-248247e2-bdf1f055",
  "ParentSeries": "95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05",
  "ParentStudy": "478cde9c-3f36815e-7253a722-f52d98c8-d15779eb",
  "Path": "/instances/518d98a6-f5a3181d-2984303f-271f3d0d-c973dfc2",
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
  "ID": "518d98a6-f5a3181d-2984303f-271f3d0d-c973dfc2",
  "ParentPatient": "64c94bda-110c6fa1-720a3d46-248247e2-bdf1f055",
  "ParentSeries": "95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05",
  "ParentStudy": "478cde9c-3f36815e-7253a722-f52d98c8-d15779eb",
  "Path": "/instances/518d98a6-f5a3181d-2984303f-271f3d0d-c973dfc2",
  "Status": "AlreadyStored"
}
```

To replace instead of keep, start the server with `PACS_OVERWRITE_INSTANCES=true`
([settings](settings.md#what-is-stored-and-how)).

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
| Patient | Patient ID | `LITS-0` |
| Study | Patient ID, Study Instance UID | `LITS-0\|2.25.3100000` |
| Series | the same, then Series Instance UID | `LITS-0\|2.25.3100000\|2.25.3200000` |
| Instance | the same, then SOP Instance UID | `LITS-0\|2.25.3100000\|2.25.3200000\|2.25.3300001` |

So a script that knows the DICOM identifiers can compute the address without asking:

```bash
printf 'LITS-0|2.25.3100000' | sha1sum
# 478cde9c3f36815e7253a722f52d98c8d15779eb
# so the study is /studies/478cde9c-3f36815e-7253a722-f52d98c8-d15779eb
```

Two consequences. The same study has the same identifier on every pacs-rs server. And a
patient is the studies that share a Patient ID: pacs-rs does not decide whether two IDs are
one person.

## List what is there

```console
$ curl http://127.0.0.1:8080/patients
```

```json
["64c94bda-110c6fa1-720a3d46-248247e2-bdf1f055"]
```

```console
$ curl http://127.0.0.1:8080/studies
```

```json
["478cde9c-3f36815e-7253a722-f52d98c8-d15779eb"]
```

With `?expand` the list holds the resources themselves instead of their identifiers. On a
large archive, read it in pages with `limit` and `since` (how many, and after how many):

```console
$ curl 'http://127.0.0.1:8080/studies?expand&limit=50&since=0'
```

```json
[
  {
    "ID": "478cde9c-3f36815e-7253a722-f52d98c8-d15779eb",
    "IsStable": false,
    "Labels": [],
    "LastUpdate": "...",
    "MainDicomTags": {
      "StudyDate": "20261117",
      "StudyTime": "120000",
      "AccessionNumber": "",
      "ReferringPhysicianName": "",
      "StudyDescription": "Three slices of LiTS volume 0",
      "StudyInstanceUID": "2.25.3100000",
      "StudyID": "1"
    },
    "ParentPatient": "64c94bda-110c6fa1-720a3d46-248247e2-bdf1f055",
    "PatientMainDicomTags": {
      "PatientName": "VOLUME 0^LITS",
      "PatientID": "LITS-0",
      "PatientBirthDate": "",
      "PatientSex": ""
    },
    "Series": ["95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05"],
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
$ curl http://127.0.0.1:8080/studies/478cde9c-3f36815e-7253a722-f52d98c8-d15779eb/series
```

```json
[
  {
    "ExpectedNumberOfInstances": null,
    "ID": "95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05",
    "Instances": [
      "518d98a6-f5a3181d-2984303f-271f3d0d-c973dfc2",
      "8a3affa1-2acb2dfe-012c8fcf-a1d113d6-bdf4a1fb",
      "9cd16b73-7d0f8d85-241e9bbc-f9ddb0a0-afb3d988"
    ],
    "IsStable": false,
    "Labels": [],
    "LastUpdate": "...",
    "MainDicomTags": {
      "Modality": "CT",
      "SeriesDescription": "Axial 5 mm",
      "SeriesInstanceUID": "2.25.3200000",
      "SeriesNumber": "1",
      "ImageOrientationPatient": "1\\0\\0\\0\\1\\0"
    },
    "ParentStudy": "478cde9c-3f36815e-7253a722-f52d98c8-d15779eb",
    "Status": "Unknown",
    "Type": "Series"
  }
]
```

The instances of a series come in the order of their Instance Number:

```console
$ curl http://127.0.0.1:8080/series/95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05/instances
```

```json
[
  {
    "FileSize": 146666,
    "ID": "518d98a6-f5a3181d-2984303f-271f3d0d-c973dfc2",
    "Labels": [],
    "MainDicomTags": {
      "SOPInstanceUID": "2.25.3300001",
      "InstanceNumber": "1",
      "ImagePositionPatient": "-172.899994\\-180\\-48",
      "ImageOrientationPatient": "1\\0\\0\\0\\1\\0"
    },
    "ParentSeries": "95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05",
    "Type": "Instance",
    "IndexInSeries": 1
  },
  {
    "FileSize": 145784,
    "ID": "8a3affa1-2acb2dfe-012c8fcf-a1d113d6-bdf4a1fb",
    "Labels": [],
    "MainDicomTags": {
      "SOPInstanceUID": "2.25.3300002",
      "InstanceNumber": "2",
      "ImagePositionPatient": "-172.899994\\-180\\-43",
      "ImageOrientationPatient": "1\\0\\0\\0\\1\\0"
    },
    "ParentSeries": "95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05",
    "Type": "Instance",
    "IndexInSeries": 2
  },
  {
    "FileSize": 143670,
    "ID": "9cd16b73-7d0f8d85-241e9bbc-f9ddb0a0-afb3d988",
    "Labels": [],
    "MainDicomTags": {
      "SOPInstanceUID": "2.25.3300003",
      "InstanceNumber": "3",
      "ImagePositionPatient": "-172.899994\\-180\\-38",
      "ImageOrientationPatient": "1\\0\\0\\0\\1\\0"
    },
    "ParentSeries": "95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05",
    "Type": "Instance",
    "IndexInSeries": 3
  }
]
```

And from any resource, the ones above it, here the patient of a series:

```console
$ curl http://127.0.0.1:8080/series/95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05/patient
```

```json
{
  "ID": "64c94bda-110c6fa1-720a3d46-248247e2-bdf1f055",
  "IsProtected": false,
  "IsStable": false,
  "Labels": [],
  "LastUpdate": "...",
  "MainDicomTags": {
    "PatientName": "VOLUME 0^LITS",
    "PatientID": "LITS-0",
    "PatientBirthDate": "",
    "PatientSex": ""
  },
  "Studies": ["478cde9c-3f36815e-7253a722-f52d98c8-d15779eb"],
  "Type": "Patient"
}
```

How much a study or a patient holds:

```console
$ curl http://127.0.0.1:8080/studies/478cde9c-3f36815e-7253a722-f52d98c8-d15779eb/statistics
```

```json
{
  "CountInstances": 3,
  "CountSeries": 1,
  "DicomDiskSize": "436120",
  "DicomDiskSizeMB": 0,
  "DicomUncompressedSize": "436120",
  "DicomUncompressedSizeMB": 0,
  "DiskSize": "436120",
  "DiskSizeMB": 0,
  "UncompressedSize": "436120",
  "UncompressedSizeMB": 0
}
```

**Why 146666 bytes, when the file sent had 525190?** pacs-rs stores uncompressed images in
JPEG-LS, which is lossless: every pixel value comes back as it was sent, and the server
checks that before it keeps the compressed copy. `FileSize` and the statistics count the
files as stored. `PACS_COMPRESSION=off` keeps them as received
([compression](dicomweb.md#compression-and-transcoding)).

## Fetch files

One instance, as the DICOM file the server keeps:

```console
$ curl -o slice.dcm http://127.0.0.1:8080/instances/518d98a6-f5a3181d-2984303f-271f3d0d-c973dfc2/file
```

A whole study or patient as a ZIP, in folders by patient, study and series
(`LITS-0 - VOLUME 0^LITS/Three slices of LiTS volume 0/1 - CT - Axial 5 mm/CT000001.dcm`).
It is sent file by file as it is built, so a large study does not wait in memory:

```console
$ curl -o study.zip http://127.0.0.1:8080/studies/478cde9c-3f36815e-7253a722-f52d98c8-d15779eb/archive
```

Several resources of any level in one ZIP, each file once:

```console
$ curl -o some.zip -d '{"Resources": ["518d98a6-f5a3181d-2984303f-271f3d0d-c973dfc2", "95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05"]}' \
    http://127.0.0.1:8080/tools/create-archive
```

The files come in the transfer syntax they are stored in. A program that needs another one,
or pixels without the file around them, asks through
[DICOMweb](dicomweb.md#compression-and-transcoding), which transcodes on request.

## Search

`POST /tools/find` takes a level and the attributes to match, and answers with identifiers:

```console
$ curl -d '{"Level": "Study", "Query": {"PatientID": "LITS-0", "StudyDate": "20261117"}}' \
    http://127.0.0.1:8080/tools/find
```

```json
["478cde9c-3f36815e-7253a722-f52d98c8-d15779eb"]
```

A value matches the way DICOM matches its attribute (PS3.4 C.2.2.2):

| In the value | Meaning | For |
|---|---|---|
| nothing special | this value: with regard to case, except for names | every attribute |
| `*` and `?` | any run of characters, and any one character | text and names |
| `from-to`, `from-`, `-to` | a range, both ends included | dates and times |
| `\` between UIDs | any of them | UIDs |
| an empty value, or `*` alone | anything | every attribute |

A name by its start, in whatever case, among the studies of November:

```console
$ curl -d '{"Level": "Study", "Query": {"PatientName": "volume*", "StudyDate": "20261101-20261130"}}' \
    http://127.0.0.1:8080/tools/find
```

```json
["478cde9c-3f36815e-7253a722-f52d98c8-d15779eb"]
```

`"Expand": true` answers with the resources, and `"Limit"` and `"Since"` page through them,
as in the lists above. What can be matched depends on the level:

| `Level` | Attributes |
|---|---|
| `Patient` | `PatientID`, `PatientName`, `PatientBirthDate`, `PatientSex` |
| `Study` | those, and `StudyInstanceUID`, `StudyDate`, `StudyTime`, `AccessionNumber`, `StudyID`, `StudyDescription`, `ReferringPhysicianName`, `ModalitiesInStudy` |
| `Series` | `SeriesInstanceUID`, which it needs |
| `Instance` | `SOPInstanceUID`, which it needs |

A search the server cannot do as asked is refused with the reason, never answered with
something else. Here, a date that is not written as DICOM writes dates:

```console
$ curl -d '{"Level": "Study", "Query": {"StudyDate": "2026-11-17"}}' \
    http://127.0.0.1:8080/tools/find
```

```json
{
  "HttpError": "Bad Request",
  "HttpStatus": 400,
  "Message": "Bad request",
  "Details": "..."
}
```

To search the series of a study by modality or number, or its instances, use DICOMweb's
search, which matches the same way ([DICOMweb](dicomweb.md)).

## Delete

Deleting a resource deletes everything under it, and every level above it that is left
empty. The answer names the closest level still there:

```console
$ curl -X DELETE http://127.0.0.1:8080/instances/518d98a6-f5a3181d-2984303f-271f3d0d-c973dfc2
```

```json
{
  "RemainingAncestor": {
    "ID": "95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05",
    "Path": "/series/95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05",
    "Type": "Series"
  }
}
```

The series still has two instances, so it stays. Deleting the study takes the series, its
instances and the patient, who has no other study:

```console
$ curl -X DELETE http://127.0.0.1:8080/studies/478cde9c-3f36815e-7253a722-f52d98c8-d15779eb
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
