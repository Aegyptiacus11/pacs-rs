# Quickstart

A server, a first study in it, and that study on screen and over HTTP: about five minutes.
You need Docker and `curl`.

The commands shown after a `$` on this page are run against a real server, with the answers
shown, each time the documentation changes ([how](building.md#the-documentations-examples)).
If one stops being true, the page cannot be published.

## 1. Start the server

```bash
curl -fsSLO https://raw.githubusercontent.com/Aegyptiacus11/pacs-rs/main/compose.yaml
docker compose up -d --wait
```

The server is at `http://127.0.0.1:8080`, reachable from this machine only. Its index and its
files are in the `pacs-data` volume, so they survive a restart.

## 2. Get a study to try

Any DICOM files you have will do. To follow this page to the letter, take the three slices of
the sample study, a 64 × 64 synthetic CT with no patient behind it:

```bash
for n in 1 2 3; do
  curl -fsSLO "https://aegyptiacus11.github.io/pacs-rs/sample/0000$n.dcm"
done
```

## 3. Send it

One file a request, to `/instances`:

```console
$ curl --data-binary @00001.dcm http://127.0.0.1:8080/instances
$ curl --data-binary @00002.dcm http://127.0.0.1:8080/instances
$ curl --data-binary @00003.dcm http://127.0.0.1:8080/instances
```

The answer to the last one:

```json
{
  "ID": "fbb232a1-b9bb23aa-d6fa59e2-a3d394f3-1e7dadde",
  "ParentPatient": "11e5f1cd-80b3a875-5b7068ee-30e57e66-ca92c84c",
  "ParentSeries": "cd952d29-4afeecc6-6399a89b-609f9514-4198538e",
  "ParentStudy": "da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9",
  "Path": "/instances/fbb232a1-b9bb23aa-d6fa59e2-a3d394f3-1e7dadde",
  "Status": "Success"
}
```

pacs-rs read the file, found its patient, study and series, and filed it under them. The
three files say they belong to one series of one study, so the server now has one patient,
one study, one series and three instances.

Dragging the files onto the web interface does the same thing: it sends each one to
`/instances`.

## 4. See it

Open <http://127.0.0.1:8080/ui/>. The study list has one line: the patient Series Synthetic
(`SYN1`), a CT study of one series and three images. Open it to get the
[viewer](web-interface.md).

![The study list of the web interface with the sample study](images/quickstart-studies.jpg)

## 5. Ask for it over HTTP

A program has two ways in. The [REST API](rest-guide.md) names things by identifiers the
server computes:

```console
$ curl http://127.0.0.1:8080/studies
```

```json
["da6c0de8-a32029ef-d216100a-7bdd4856-3ea9bed9"]
```

[DICOMweb](dicomweb.md), the standard's own HTTP interface (PS3.18), names them by their
DICOM UIDs and answers in DICOM JSON. It is the interface web viewers are written for:

```console
$ curl -H 'Accept: application/dicom+json' http://127.0.0.1:8080/dicom-web/studies/2.25.1100000/series
```

```json
[
  {
    "00080020": {"vr": "DA", "Value": ["20260101"]},
    "00080030": {"vr": "TM", "Value": ["120000"]},
    "00080050": {"vr": "SH", "Value": ["ACC1"]},
    "00080090": {"vr": "PN", "Value": [{"Alphabetic": "REFERRING^PHYSICIAN"}]},
    "00081030": {"vr": "LO", "Value": ["Synthetic study"]},
    "00100010": {"vr": "PN", "Value": [{"Alphabetic": "SYNTHETIC^SERIES"}]},
    "00100020": {"vr": "LO", "Value": ["SYN1"]},
    "00100030": {"vr": "DA", "Value": ["19700101"]},
    "00100040": {"vr": "CS", "Value": ["O"]},
    "0020000D": {"vr": "UI", "Value": ["2.25.1100000"]},
    "00200010": {"vr": "SH", "Value": ["S1"]},
    "00080060": {"vr": "CS", "Value": ["CT"]},
    "0008103E": {"vr": "LO", "Value": ["Synthetic series 1"]},
    "0020000E": {"vr": "UI", "Value": ["2.25.1200000"]},
    "00200011": {"vr": "IS", "Value": [1]},
    "00201209": {"vr": "IS", "Value": [3]},
    "00081190": {"vr": "UR", "Value": ["..."]},
    "00080005": {"vr": "CS", "Value": ["ISO_IR 192"]}
  }
]
```

`"..."` on these pages stands for a value that changes from one server to the next: here
the address the series can be retrieved from.

## 6. Stop it

```bash
docker compose down        # the study is kept in the pacs-data volume
docker compose down -v     # the study is deleted with the volume
```

## Before real data

**Anyone who can reach this server can read and delete everything on it.** The quickstart is
safe because the port is published on this machine only. Before a real patient's study goes
in, turn the [login](login.md) on, and read which of the [two ways to use
pacs-rs](two-ways.md) is yours.
