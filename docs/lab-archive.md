# Lab 2: store and search

You will hand DICOM files to a PACS and watch what it does with them: where it files them,
what it refuses, and how a program finds them again.

- **Before you start:** the [set-up](labs.md#set-up-once) of the labs, with the server
  running and empty. `curl http://127.0.0.1:8080/studies` answers `[]`.
- **Read with it:** [DICOM: files and studies](dicom-files.md), the part on patient, study,
  series and instance.

## 1. Send one file

```bash
curl --data-binary @00001.dcm http://127.0.0.1:8080/instances
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

The server prints its answer on one line; it is laid out here to be read. You sent one file
and said nothing else, and the server answers with a patient, a study and a series. It read
them in the file.

## 2. Send the second file

```bash
curl --data-binary @00002.dcm http://127.0.0.1:8080/instances
```

```json
{
  "ID": "8a3affa1-2acb2dfe-012c8fcf-a1d113d6-bdf4a1fb",
  "ParentPatient": "64c94bda-110c6fa1-720a3d46-248247e2-bdf1f055",
  "ParentSeries": "95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05",
  "ParentStudy": "478cde9c-3f36815e-7253a722-f52d98c8-d15779eb",
  "Path": "/instances/8a3affa1-2acb2dfe-012c8fcf-a1d113d6-bdf4a1fb",
  "Status": "Success"
}
```

**Question.** Compare the two answers. What changed, what did not, and why?

<details>
<summary>Answer</summary>

Only `ID` and `Path` changed: this is another instance. `ParentPatient`, `ParentStudy` and
`ParentSeries` are the same as before, because the second file names the same Patient ID,
Study Instance UID and Series Instance UID as the first
([Lab 1, step 5](lab-file.md#5-three-files-one-series)). The server filed it beside the
first one without being told to.

</details>

## 3. Send the same file again

A network fails, a script is run twice: the same file often arrives more than once.

```bash
curl --data-binary @00001.dcm http://127.0.0.1:8080/instances
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

The status is `AlreadyStored`, and the `ID` is the one of step 1. The server recognised the
instance by its SOP Instance UID and kept the copy it had.

## 4. Send something that is not DICOM

```bash
head -c 1000 /dev/urandom > notdicom.bin
curl --data-binary @notdicom.bin http://127.0.0.1:8080/instances
```

```json
{
  "HttpError": "Bad Request",
  "HttpStatus": 400,
  "Message": "Bad file format",
  "Details": "Cannot parse an invalid DICOM file (size: 1000 bytes): not a readable DICOM file: Could not parse meta group data set"
}
```

Nothing was stored. An archive that kept whatever it was sent would hand unreadable files to
a radiologist months later.

## 5. Search by what is in the files

So far you have used pacs-rs's own REST API. Now ask with **DICOMweb**, the DICOM standard's
HTTP interface, which every web viewer speaks. Search for the studies of the patient
`LITS-0`:

```bash
curl "http://127.0.0.1:8080/dicom-web/studies?PatientID=LITS-0"
```

```json
[
  {
    "00080020": {"vr": "DA", "Value": ["20261117"]},
    "00080030": {"vr": "TM", "Value": ["120000"]},
    "00080050": {"vr": "SH"},
    "00080090": {"vr": "PN"},
    "00081030": {"vr": "LO", "Value": ["Three slices of LiTS volume 0"]},
    "00100010": {"vr": "PN", "Value": [{"Alphabetic": "VOLUME 0^LITS"}]},
    "00100020": {"vr": "LO", "Value": ["LITS-0"]},
    "00100030": {"vr": "DA"},
    "00100040": {"vr": "CS"},
    "0020000D": {"vr": "UI", "Value": ["2.25.3100000"]},
    "00200010": {"vr": "SH", "Value": ["1"]},
    "00080061": {"vr": "CS", "Value": ["CT"]},
    "00201206": {"vr": "IS", "Value": [1]},
    "00201208": {"vr": "IS", "Value": [2]},
    "00081190": {"vr": "UR", "Value": ["http://127.0.0.1:8080/dicom-web/studies/2.25.3100000"]},
    "00080005": {"vr": "CS", "Value": ["ISO_IR 192"]}
  }
]
```

One study, described by its DICOM elements. Two of them are in no file: the server counted
them. `00201206` is the number of series in the study (1) and `00201208` the number of
instances (2).

Now search for a patient who is not there:

```bash
curl "http://127.0.0.1:8080/dicom-web/studies?PatientID=NOBODY"
```

```json
[]
```

**Question.** A search that finds nothing answers an empty list, not an error. Why does that
matter to a program?

<details>
<summary>Answer</summary>

"No study matches" and "the search failed" are different facts, and a program must be able to
tell them apart. A hospital system that asks for a patient's earlier studies and gets an
error should try again or warn someone; one that gets an empty list knows there are none.

</details>

## 6. Complete the study

Send the third slice, then ask again, this time with a wild card on the patient's name:

```bash
curl --data-binary @00003.dcm http://127.0.0.1:8080/instances
curl "http://127.0.0.1:8080/dicom-web/studies?PatientName=VOL*"
```

The first answer is a `Success` like the others. In the second, find `00201208`:

```json
"00201208": {"vr": "IS", "Value": [3]}
```

The study has grown to three instances. A study in a PACS is never declared complete by the
sender: it is whatever has arrived so far.

## 7. Two names for one study

The REST API and DICOMweb name the same study differently. List the studies with the REST
API, then ask for the one it names:

```bash
curl http://127.0.0.1:8080/studies
curl http://127.0.0.1:8080/studies/478cde9c-3f36815e-7253a722-f52d98c8-d15779eb
```

```json
["478cde9c-3f36815e-7253a722-f52d98c8-d15779eb"]
```

```json
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
```

`"..."` stands for the time of your last upload.

**Question.** Your server gave this study the same identifier, `478cde9c-…`, as the server
these pages were written with. How can two servers that never met agree on it?

<details>
<summary>Answer</summary>

The identifier is not drawn at random or counted up. It is computed from what the files say
about the study, so any server that receives these files computes the same one
([Identifiers](rest-guide.md#identifiers)). The Study Instance UID, `2.25.3100000`, is the
name DICOM itself gives the study, and it is the one DICOMweb uses.

</details>

## 8. A second patient

Every file so far belonged to one patient. Make a file for another: copy the first slice,
change the patient, and give the copy new UIDs at every level. Save this as
`second_patient.py`:

```python
import pydicom
from pydicom.uid import generate_uid

ds = pydicom.dcmread("00001.dcm")
ds.PatientID = "LAB-1"
ds.PatientName = "STUDENT^LAB"
# a new examination needs a new name at every level; prefix=None makes 2.25. UIDs
ds.StudyInstanceUID = generate_uid(prefix=None)
ds.SeriesInstanceUID = generate_uid(prefix=None)
ds.SOPInstanceUID = generate_uid(prefix=None)
ds.file_meta.MediaStorageSOPInstanceUID = ds.SOPInstanceUID
ds.save_as("second.dcm")
print("study", ds.StudyInstanceUID)
```

Run it, send the new file, and list the patients:

```bash
uv run --with pydicom python second_patient.py
curl --data-binary @second.dcm http://127.0.0.1:8080/instances
curl http://127.0.0.1:8080/patients
```

The script prints the UID it made, which is different on every run. The upload answers
`Success` with identifiers of its own, and the last command now lists two patients:

```json
["64c94bda-110c6fa1-720a3d46-248247e2-bdf1f055", "81ab065b-94925ab0-f37ac7c9-46b8c193-2fd0c87d"]
```

Open <http://127.0.0.1:8080/ui/> to see both in the study list.

**Question.** The script changes the three UIDs as well as the patient. What would be wrong
with a file that kept the UIDs of `00001.dcm` under the new Patient ID?

<details>
<summary>Answer</summary>

Two different objects would then carry one name. The file would claim to be instance
`2.25.3300001` of study `2.25.3100000`, which the server already holds for another patient. A
UID must name one thing in the world, and every DICOM system relies on that when it decides
where a file goes. A copy that is meant to be a new examination needs new UIDs;
[de-identification](de-identification.md) replaces them for the same reason.

</details>

## What you learned

- A PACS files an instance by the identifiers written in it. The sender never says where it
  goes.
- An instance that arrives twice is stored once. A file that is not DICOM is refused.
- A study is whatever has arrived of it so far.
- The same study has a server-computed identifier in the REST API and its Study Instance UID
  in DICOMweb.

Keep the server as it is: [Lab 3: DICOM on the network](lab-network.md) continues with these
studies.
