# DICOM: files and studies

A CT scanner does not produce pictures the way a camera does. It produces DICOM files, and a
PACS is the server that keeps them. This page opens one of those files and shows what is
inside, then how hundreds of files add up to one examination.

You need no background in medical imaging. Every example uses the three files of the
[quickstart](quickstart.md), and [Lab 1](lab-file.md) lets you repeat all of it with your own
hands.

## One file holds one image and its story

A DICOM file holds one object. Most often that object is one image, together with everything
known about it: whose it is, when and on which machine it was made, how to read its pixels,
and the pixels themselves. The standard calls such an object a *SOP Instance*. These pages
say **instance**.

A JPEG from a phone separates the picture from a few optional notes about it. In DICOM the
notes are the larger half of the design: an image without its patient and its geometry is of
no use to a radiologist, so the two travel in one file and cannot drift apart.

The file `00001.dcm` of the sample study is 525190 bytes long and has four parts
(PS3.10 Section 7.1):

| Bytes | What is there |
|---|---|
| 0 to 127 | the preamble: 128 bytes that mean nothing to DICOM, all zero here |
| 128 to 131 | the four letters `DICM` |
| 132 to 297 | the File Meta Information: what kind of object follows and how it is written |
| 298 to the end | the data set: the object itself, with the pixels last |

You can check the second line yourself:

```bash
xxd -s 128 -l 4 00001.dcm
# 00000080: 4449 434d                                DICM
```

A file without `DICM` at byte 128 is not a DICOM file. pacs-rs refuses it with
`Bad file format`.

## A file is a list of facts

Everything after `DICM` is a list of **data elements**. One element is one fact, written in
four fields (PS3.5 Section 7.1):

| Field | In this example | Meaning |
|---|---|---|
| Tag | `(0008,0020)` | which fact this is: a group number and an element number, in hexadecimal. This tag is the Study Date |
| Value Representation (VR) | `DA` | the type of the value. `DA` is a date, `PN` a person's name, `UI` a unique identifier, `US` an unsigned 16-bit number |
| Length | `8` | how many bytes the value takes |
| Value | `20261117` | the fact itself |

In the file, that element is sixteen bytes. Numbers are written with their low byte first:

```text
08 00 20 00   44 41   08 00   32 30 32 36 31 31 31 37
(0008,0020)   D  A    8       2  0  2  6  1  1  1  7
```

Two rules make a data set easy to read and to check (PS3.5 Section 7.1):

- The elements come in increasing order of tag.
- A tag appears at most once.

What each tag means is fixed by the standard's dictionary (PS3.6). `(0010,0010)` is the
patient's name in every DICOM file ever written, whichever company built the scanner. A
vendor's own facts go in *private* elements, which live in the odd-numbered groups
(PS3.5 Section 7.8). pacs-rs keeps private elements as it receives them.

### Reading the elements of a stored instance

Once a file is in pacs-rs, you can ask for its elements without its pixels. Send the file,
then ask DICOMweb for the instance's metadata:

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

This is the data set of the file, written as JSON (PS3.18 Annex F). Each entry is one
element: the tag without its brackets, the VR, and the values.

Read down the list and the file describes itself:

| Element | Value | What it tells you |
|---|---|---|
| `00080060` Modality | `CT` | the image comes from a CT scanner |
| `00280010`, `00280011` Rows, Columns | `512`, `512` | the image is 512 pixels high and 512 wide |
| `00280002`, `00280004` | `1`, `MONOCHROME2` | one number per pixel, shown as shades of grey where higher is brighter |
| `00280100`, `00280101` Bits Allocated, Bits Stored | `16`, `16` | each pixel takes 16 bits, and all of them are used |
| `00280103` Pixel Representation | `1` | the numbers are signed, so a pixel can be negative |
| `00280030` Pixel Spacing | `0.703125` | one pixel covers 0.703125 mm of the patient |

An element with a VR and no value, such as the Accession Number (`00080050`), is present in
the file and empty. The dataset this scan comes from does not give it.

## The pixels, and how they are written

The last element of the data set is `(7FE0,0010)` Pixel Data. In the sample file it holds
512 × 512 pixels of two bytes each: 524288 bytes out of the file's 525190. Everything else in
the file fits in under a kilobyte.

How the elements and the pixels are turned into bytes is the file's **transfer syntax**. The
File Meta Information names it with a UID. A transfer syntax settles three things
(PS3.5 Section 10):

- whether each element writes its VR (*explicit*) or leaves it to the dictionary (*implicit*);
- the byte order of numbers;
- whether the pixels are compressed, and with which method.

| Transfer syntax | UID | Notes |
|---|---|---|
| Implicit VR Little Endian | `1.2.840.10008.1.2` | the default every DICOM system must support (PS3.5 Section 10.1) |
| Explicit VR Little Endian | `1.2.840.10008.1.2.1` | how the sample files are written; not compressed |
| JPEG-LS Lossless | `1.2.840.10008.1.2.4.80` | compressed, with every pixel value kept |

The File Meta Information itself is always written in Explicit VR Little Endian, whatever
follows it (PS3.10 Section 7.1). A program can therefore read the transfer syntax first and
then know how to read the rest.

> [!NOTE]
> **What pacs-rs does.** An image that arrives uncompressed is stored as JPEG-LS Lossless,
> after a check that decoding it gives back every pixel. An image that arrives compressed is
> kept as it came. A client that wants another transfer syntax asks for it, and pacs-rs
> converts on the way out ([compression and transcoding](dicomweb.md#compression-and-transcoding)).
> [Lab 4](lab-pixels.md) measures this on the sample study.

## Patient, study, series, instance

One CT examination is hundreds of files. Nothing outside the files says they belong together.
Each file repeats the facts about its patient, its study and its series, and files that carry
the same identifiers are one examination (PS3.3 Section A.1.2).

| Level | What it is | Identified by | In the sample |
|---|---|---|---|
| Patient | a person | `(0010,0020)` Patient ID | `LITS-0` |
| Study | one examination of that person: one visit to the scanner | `(0020,000D)` Study Instance UID | `2.25.3100000` |
| Series | one acquisition within the study, from one device | `(0020,000E)` Series Instance UID | `2.25.3200000` |
| Instance | one object, most often an image | `(0008,0018)` SOP Instance UID | `2.25.3300001` to `2.25.3300003` |

A study belongs to exactly one patient, and a series to exactly one study. A server never
has to be told where a file goes: it reads the four identifiers and files it. That is what
happened in the quickstart, when three files sent one by one became one study.

> [!NOTE]
> **What pacs-rs does.**
>
> - It keeps the attributes of each level once in its index, as the latest file to arrive
>   gave them. They are the `MainDicomTags` of the
>   [REST API](rest-guide.md#list-what-is-there).
> - A file whose SOP Instance UID is already stored is a second copy of the same instance.
>   The first copy is kept, unless `PACS_OVERWRITE_INSTANCES` says otherwise.

> [!CAUTION]
> **A Patient ID is only as unique as the hospital that gave it.** Two hospitals can both
> have a patient `12345`. pacs-rs treats one Patient ID as one patient. It does not merge or
> split patients by their names or birth dates.

## UIDs

Studies, series and instances are named by **unique identifiers**: numbers separated by
dots, 64 characters at most, where no number starts with a zero unless it is the single digit
`0` (PS3.5 Section 9.1). A UID is meant to be unique in the world, not only inside one
hospital. That is why a study can move from one archive to another and keep its name.

A UID begins with the root of whoever made it:

- `1.2.840.10008` is DICOM's own root. Every UID under it names something the standard
  defines, such as the transfer syntaxes above, or `1.2.840.10008.5.1.4.1.1.2`, which is
  CT Image Storage, the kind of object the sample files hold.
- `2.25.` followed by one large number is the root for UIDs made from a UUID, for those who
  have no root of their own (PS3.5 Annex B.2). The sample uses it with small numbers so that
  its UIDs are easy to read. The dataset the sample comes from has no UIDs, so these were
  given when its slices were written as DICOM.

> [!CAUTION]
> **A UID carries no meaning.** Do not read a date or a hospital out of one, and never edit
> one by hand. Two different objects with one UID is the error DICOM systems forgive least.
> [De-identification](de-identification.md) replaces UIDs for a related reason: they can
> point back to the hospital that made them.

## Where next

- Open a file yourself and find these elements: [Lab 1: inside a DICOM file](lab-file.md).
- How two DICOM systems send each other these files: [DICOM: the network](dicom-network.md).
- The same data over HTTP: [DICOMweb](dicomweb.md) and the
  [REST API, task by task](rest-guide.md).
- Removing the patient from the elements: [De-identification](de-identification.md).
