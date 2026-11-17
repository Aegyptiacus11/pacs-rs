# DICOM: files and studies

What is in a DICOM file, how files add up to a study, and what pacs-rs does with each part.
Enough to read the rest of this book; the standard's own sections are named for the details.
Every example is the sample study of the [quickstart](quickstart.md).

## One file, one image

A DICOM file holds one object, most often one image, together with everything known about
it: whose it is, when and how it was made, and the pixels. The standard calls the object a
*SOP Instance*; this book says *instance*.

The file `00001.dcm` of the sample has 525190 bytes, in three parts (PS3.10 Section 7.1):

| Bytes | What |
|---|---|
| 0 to 127 | the preamble: 128 bytes with no meaning to DICOM, all zero here |
| 128 to 131 | the four letters `DICM` |
| 132 to 297 | the File Meta Information: what kind of object follows and how it is encoded |
| 298 to the end | the data set: the object itself, pixels last |

```bash
xxd -s 128 -l 4 00001.dcm
# 00000080: 4449 434d                                DICM
```

A file without `DICM` at byte 128 is not a DICOM file, and pacs-rs refuses it
(`Bad file format`).

## Data elements

Everything after `DICM` is a list of *data elements*. Each is one fact, in four fields
(PS3.5 Section 7.1):

| Field | Here | Meaning |
|---|---|---|
| Tag | `(0008,0020)` | which fact: a group and an element number, both in hexadecimal. This one is Study Date |
| Value Representation (VR) | `DA` | the type of the value: `DA` is a date, `PN` a person's name, `UI` a unique identifier, `US` an unsigned 16-bit number |
| Length | `8` | bytes in the value |
| Value | `20261117` | the fact itself |

In the file those are sixteen bytes, the numbers with their low byte first:

```text
08 00 20 00   44 41   08 00   32 30 32 36 31 31 31 37
(0008,0020)   D  A    8       2  0  2  6  1  1  1  7
```

Two rules make a data set easy to read and to check: the elements come in increasing order
of tag, and a tag appears at most once (PS3.5 Section 7.1).

The tags are fixed by the standard's dictionary (PS3.6): `(0010,0010)` is the patient's name
in every DICOM file ever written. A vendor's own facts go in *private* elements, in the odd
groups (PS3.5 Section 7.8); pacs-rs keeps them as received.

To see the elements of an instance without its pixels, ask DICOMweb for its metadata:

```console
$ curl --data-binary @00001.dcm http://127.0.0.1:8080/instances
$ curl http://127.0.0.1:8080/dicom-web/studies/2.25.3100000/series/2.25.3200000/instances/2.25.3300001/metadata
```

```json
[
  {
    "00080008": {"vr": "CS", "Value": ["DERIVED", "SECONDARY", "AXIAL"]},
    "00080016": {"vr": "UI", "Value": ["1.2.840.10008.5.1.4.1.1.2"]},
    "00080018": {"vr": "UI", "Value": ["2.25.3300001"]},
    "00080020": {"vr": "DA", "Value": ["20261117"]},
    "00080030": {"vr": "TM", "Value": ["120000"]},
    "00080050": {"vr": "SH"},
    "00080060": {"vr": "CS", "Value": ["CT"]},
    "00080090": {"vr": "PN"},
    "00081030": {"vr": "LO", "Value": ["Three slices of LiTS volume 0"]},
    "0008103E": {"vr": "LO", "Value": ["Axial 5 mm"]},
    "00100010": {"vr": "PN", "Value": [{"Alphabetic": "VOLUME 0^LITS"}]},
    "00100020": {"vr": "LO", "Value": ["LITS-0"]},
    "00100030": {"vr": "DA"},
    "00100040": {"vr": "CS"},
    "00180050": {"vr": "DS", "Value": ["5"]},
    "00185100": {"vr": "CS", "Value": ["HFS"]},
    "0020000D": {"vr": "UI", "Value": ["2.25.3100000"]},
    "0020000E": {"vr": "UI", "Value": ["2.25.3200000"]},
    "00200010": {"vr": "SH", "Value": ["1"]},
    "00200011": {"vr": "IS", "Value": [1]},
    "00200013": {"vr": "IS", "Value": [1]},
    "00200032": {"vr": "DS", "Value": ["-172.899994", "-180", "-48"]},
    "00200037": {"vr": "DS", "Value": ["1", "0", "0", "0", "1", "0"]},
    "00200052": {"vr": "UI", "Value": ["2.25.3200000.1"]},
    "00201041": {"vr": "DS", "Value": ["-48"]},
    "00280002": {"vr": "US", "Value": [1]},
    "00280004": {"vr": "CS", "Value": ["MONOCHROME2"]},
    "00280010": {"vr": "US", "Value": [512]},
    "00280011": {"vr": "US", "Value": [512]},
    "00280030": {"vr": "DS", "Value": ["0.703125", "0.703125"]},
    "00280100": {"vr": "US", "Value": [16]},
    "00280101": {"vr": "US", "Value": [16]},
    "00280102": {"vr": "US", "Value": [15]},
    "00280103": {"vr": "US", "Value": [1]},
    "00281050": {"vr": "DS", "Value": ["40"]},
    "00281051": {"vr": "DS", "Value": ["400"]},
    "00281052": {"vr": "DS", "Value": ["0"]},
    "00281053": {"vr": "DS", "Value": ["1"]},
    "00281054": {"vr": "LO", "Value": ["HU"]}
  }
]
```

This is the same data set written as JSON (PS3.18 Annex F): the tag without its brackets,
the VR, the values. Read down it and the file describes itself: a CT image (`00080060`),
512 rows by 512 columns (`00280010`, `00280011`), one sample a pixel, shown as shades of grey
(`00280002`, `00280004`), each pixel stored in 16 bits, all of them used, as a signed number
(`00280100`, `00280101`, `00280103`). An element with a VR and no value, such as the
Accession Number (`00080050`), is in the file and empty: the dataset the scan comes from
does not give it.

## Pixels and transfer syntaxes

The last element, `(7FE0,0010)` Pixel Data, holds the image: here 512 × 512 pixels of two
bytes, 524288 bytes of the file's 525190.

How the elements and the pixels are written as bytes is the file's *transfer syntax*, named
by a UID in the File Meta Information. Three things vary: whether the VR is written
(explicit) or left to the dictionary (implicit), the byte order, and whether the pixels are
compressed and how (PS3.5 Section 10).

| Transfer syntax | UID | |
|---|---|---|
| Implicit VR Little Endian | `1.2.840.10008.1.2` | the default every DICOM system must support (PS3.5 Section 10.1) |
| Explicit VR Little Endian | `1.2.840.10008.1.2.1` | how the sample files are written; uncompressed |
| JPEG-LS Lossless | `1.2.840.10008.1.2.4.80` | compressed, every pixel value kept |

**What pacs-rs does:** the File Meta Information is always Explicit VR Little Endian,
whatever follows it (PS3.10 Section 7.1), so a file's transfer syntax can be read before
anything else. An image that arrives uncompressed is stored as JPEG-LS Lossless, after a
check that decoding gives back every pixel; an image that arrives compressed is kept as it
came. A client that wants another transfer syntax asks for it and pacs-rs converts on the way
out ([compression and transcoding](dicomweb.md#compression-and-transcoding)).

## Patient, study, series, instance

A CT scan is hundreds of files. What makes them one scan is inside them: every file repeats
the facts about its patient, its study and its series, and files with the same identifiers
belong together (PS3.3 Section A.1.2).

| Level | What it is | Identified by | In the sample |
|---|---|---|---|
| Patient | a person | `(0010,0020)` Patient ID | `LITS-0` |
| Study | one examination of that person: one visit to the scanner | `(0020,000D)` Study Instance UID | `2.25.3100000` |
| Series | one acquisition within the study, from one device | `(0020,000E)` Series Instance UID | `2.25.3200000` |
| Instance | one object: an image, most often | `(0008,0018)` SOP Instance UID | `2.25.3300001` to `...3` |

A study belongs to exactly one patient and a series to exactly one study. So a server never
needs to be told where a file goes: it reads the four identifiers and files it. That is
what happened when the quickstart sent three files and got one study.

**What pacs-rs does:**

- The index keeps the attributes of each level once (the `MainDicomTags` of the
  [REST API](rest-guide.md#list-what-is-there)), as the latest file to arrive gave
  them.
- A file whose SOP Instance UID is already stored is a second copy of the same instance: the
  first is kept, unless `PACS_OVERWRITE_INSTANCES` says otherwise.
- **The Patient ID is only as unique as the hospital that gave it.** Two hospitals can both
  have a patient `12345`. pacs-rs treats one Patient ID as one patient and does not merge
  or split on names or birth dates.

## UIDs

Studies, series and instances are named by *unique identifiers*: numbers separated by dots,
at most 64 characters, where no number starts with a zero unless it is the single digit `0`
(PS3.5 Section 9.1). They are meant to be unique in the world, not only in one hospital,
which is why a study can move between archives and keep its name.

A UID starts with the root of whoever made it. `1.2.840.10008` is DICOM's own: every UID
under it names something the standard defines, such as the transfer syntaxes above or
`1.2.840.10008.5.1.4.1.1.2`, CT Image Storage, the kind of object the sample files hold.
`2.25.` followed by one large number is the root for UIDs made from a UUID, for those who
have no root of their own (PS3.5 Annex B.2); the sample's UIDs use it with small numbers so
that they can be read. The dataset the sample comes from has no UIDs of its own: these were
given when its slices were written as DICOM.

**A UID carries no meaning.** Do not read a date or a hospital out of one, and never edit
one by hand: two different objects with one UID is the error DICOM systems forgive least.
[De-identification](de-identification.md) replaces UIDs for exactly this reason: they can
point back to the hospital that made them.

## Where next

- How two DICOM systems exchange these files: [DICOM: the network](dicom-network.md).
- The same data over HTTP: [DICOMweb](dicomweb.md) and the [REST API, task by task](rest-guide.md).
- Removing the patient from the elements: [De-identification](de-identification.md).
