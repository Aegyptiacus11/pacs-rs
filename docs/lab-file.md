# Lab 1: inside a DICOM file

You will take one DICOM file apart: first as raw bytes, then with a library that reads it for
you. At the end you can say what is in a DICOM file and where.

- **Before you start:** the [set-up](labs.md#set-up-once) of the labs. This lab uses only the
  file `00001.dcm`; the server is not needed.
- **Read with it:** [DICOM: files and studies](dicom-files.md).

## 1. The marker

A DICOM file begins with 128 bytes that mean nothing, then four letters. Print the first 16
bytes, then the four bytes at position 128:

```bash
xxd -l 16 00001.dcm
xxd -s 128 -l 4 00001.dcm
```

```text
00000000: 0000 0000 0000 0000 0000 0000 0000 0000  ................
00000080: 4449 434d                                DICM
```

`-s 128` skips to byte 128 and `-l 4` prints four bytes. The left column is the position in
hexadecimal: `80` is 128.

**Question.** Why would a format begin with 128 bytes it does not use?

<details>
<summary>Answer</summary>

So that a file can be two things at once. The preamble is left for whoever writes the file to
use as they wish (PS3.10 Section 7.1): another format's header can sit there, and a program
that knows nothing of DICOM can then open the same file its own way. A DICOM reader skips
those bytes and looks for `DICM`.

</details>

## 2. The label on the box

After `DICM` comes the File Meta Information: a few elements that say what the file holds and
how it is written. Print them:

```bash
xxd -s 132 -l 166 00001.dcm
```

```text
00000084: 0200 0000 554c 0400 9a00 0000 0200 0100  ....UL..........
00000094: 4f42 0000 0200 0000 0001 0200 0200 5549  OB............UI
000000a4: 1a00 312e 322e 3834 302e 3130 3030 382e  ..1.2.840.10008.
000000b4: 352e 312e 342e 312e 312e 3200 0200 0300  5.1.4.1.1.2.....
000000c4: 5549 0c00 322e 3235 2e33 3330 3030 3031  UI..2.25.3300001
000000d4: 0200 1000 5549 1400 312e 322e 3834 302e  ....UI..1.2.840.
000000e4: 3130 3030 382e 312e 322e 3100 0200 1200  10008.1.2.1.....
000000f4: 5549 1c00 312e 322e 3832 362e 302e 312e  UI..1.2.826.0.1.
00000104: 3336 3830 3034 332e 382e 3439 382e 3100  3680043.8.498.1.
00000114: 0200 1300 5348 0e00 5059 4449 434f 4d20  ....SH..PYDICOM 
00000124: 332e 302e 3120                           3.0.1 
```

Read the first element by hand. Numbers are written low byte first, so `0200 0000` is group
`0002`, element `0000`:

| Bytes | Field | Value |
|---|---|---|
| `02 00 00 00` | tag | `(0002,0000)` File Meta Information Group Length |
| `55 4c` | VR | `UL`, an unsigned 32-bit number |
| `04 00` | length | 4 bytes |
| `9a 00 00 00` | value | hexadecimal 9a = 154 |

So 154 bytes of File Meta Information follow this element. The element itself ends at byte
144, and 144 + 154 = 298: the data set begins at byte 298.

**Question.** Three UIDs can be read in the right-hand column. One of them is
`1.2.840.10008.1.2.1`. Which element holds it, and what does it tell a reader?

<details>
<summary>Answer</summary>

It follows the bytes `0200 1000 5549`: tag `(0002,0010)`, VR `UI`. That is the Transfer
Syntax UID, and `1.2.840.10008.1.2.1` is Explicit VR Little Endian. It tells a reader how
everything from byte 298 on is written: each element states its VR, numbers have their low
byte first, and the pixels are not compressed.

The other two are `1.2.840.10008.5.1.4.1.1.2`, the kind of object (CT Image Storage), and
`2.25.3300001`, the name of this one instance.

</details>

## 3. One fact, sixteen bytes

The Study Date of this file is 17 November 2026, written `20261117`. Find where:

```bash
grep -abo 20261117 00001.dcm
```

```text
392:20261117
```

The value starts at byte 392. An element's tag, VR and length take eight bytes before its
value, so print from byte 384:

```bash
xxd -s 384 -l 16 00001.dcm
```

```text
00000180: 0800 2000 4441 0800 3230 3236 3131 3137  .. .DA..20261117
```

**Question.** Split those sixteen bytes into tag, VR, length and value.

<details>
<summary>Answer</summary>

| Bytes | Field | Value |
|---|---|---|
| `08 00 20 00` | tag | `(0008,0020)` Study Date |
| `44 41` | VR | `DA`, a date |
| `08 00` | length | 8 bytes |
| `32 30 32 36 31 31 31 37` | value | the characters `20261117` |

</details>

## 4. Let a library read it

Nobody reads CT scans with `xxd`. [pydicom](https://pydicom.github.io/) is the Python library
most research code uses for it, and it has a command that prints a file:

```bash
uvx pydicom show 00001.dcm
```

```text
Dataset.file_meta -------------------------------
(0002,0000) File Meta Information Group Length  UL: 154
(0002,0001) File Meta Information Version       OB: b'\x00\x01'
(0002,0002) Media Storage SOP Class UID         UI: CT Image Storage
(0002,0003) Media Storage SOP Instance UID      UI: 2.25.3300001
(0002,0010) Transfer Syntax UID                 UI: Explicit VR Little Endian
(0002,0012) Implementation Class UID            UI: 1.2.826.0.1.3680043.8.498.1
(0002,0013) Implementation Version Name         SH: 'PYDICOM 3.0.1'
-------------------------------------------------
(0008,0008) Image Type                          CS: ['DERIVED', 'SECONDARY', 'AXIAL']
(0008,0016) SOP Class UID                       UI: CT Image Storage
(0008,0018) SOP Instance UID                    UI: 2.25.3300001
(0008,0020) Study Date                          DA: '20261117'
(0008,0030) Study Time                          TM: '120000'
(0008,0050) Accession Number                    SH: ''
(0008,0060) Modality                            CS: 'CT'
(0008,0090) Referring Physician's Name          PN: ''
(0008,1030) Study Description                   LO: 'Three slices of LiTS volume 0'
(0008,103E) Series Description                  LO: 'Axial 5 mm'
(0010,0010) Patient's Name                      PN: 'VOLUME 0^LITS'
(0010,0020) Patient ID                          LO: 'LITS-0'
(0010,0030) Patient's Birth Date                DA: ''
(0010,0040) Patient's Sex                       CS: ''
(0018,0050) Slice Thickness                     DS: '5'
(0018,5100) Patient Position                    CS: 'HFS'
(0020,000D) Study Instance UID                  UI: 2.25.3100000
(0020,000E) Series Instance UID                 UI: 2.25.3200000
(0020,0010) Study ID                            SH: '1'
(0020,0011) Series Number                       IS: '1'
(0020,0013) Instance Number                     IS: '1'
(0020,0032) Image Position (Patient)            DS: [-172.899994, -180, -48]
(0020,0037) Image Orientation (Patient)         DS: [1, 0, 0, 0, 1, 0]
(0020,0052) Frame of Reference UID              UI: 2.25.3200000.1
(0020,1041) Slice Location                      DS: '-48'
(0028,0002) Samples per Pixel                   US: 1
(0028,0004) Photometric Interpretation          CS: 'MONOCHROME2'
(0028,0010) Rows                                US: 512
(0028,0011) Columns                             US: 512
(0028,0030) Pixel Spacing                       DS: [0.703125, 0.703125]
(0028,0100) Bits Allocated                      US: 16
(0028,0101) Bits Stored                         US: 16
(0028,0102) High Bit                            US: 15
(0028,0103) Pixel Representation                US: 1
(0028,1050) Window Center                       DS: '40'
(0028,1051) Window Width                        DS: '400'
(0028,1052) Rescale Intercept                   DS: '0'
(0028,1053) Rescale Slope                       DS: '1'
(0028,1054) Rescale Type                        LO: 'HU'
(7FE0,0010) Pixel Data                          OW: Array of 524288 elements
```

The first block is what you read by hand in step 2. pydicom shows names where the file has
UIDs: `CT Image Storage` is `1.2.840.10008.5.1.4.1.1.2`.

One element can be asked for by its name:

```bash
uvx pydicom show 00001.dcm::PatientID
```

```text
LITS-0
```

**Questions.**

1. Look at the tags in the left column. What do you notice about their order?
2. The Pixel Data has 524288 bytes. Find the elements that explain that number.
3. Which elements say which patient, study and series this image belongs to?

<details>
<summary>Answers</summary>

1. They only increase. A data set lists its elements in increasing order of tag, and a tag
   appears once (PS3.5 Section 7.1). A reader can stop as soon as it has passed the tag it
   wants.
2. Rows `512` × Columns `512` × Samples per Pixel `1` × Bits Allocated `16` ÷ 8 = 524288.
   The file is 525190 bytes, so everything that is not pixels takes 902 bytes.
3. Patient ID `(0010,0020)`, Study Instance UID `(0020,000D)` and Series Instance UID
   `(0020,000E)`. The image's own name is the SOP Instance UID `(0008,0018)`.

</details>

## 5. Three files, one series

Print the identifiers of the three files side by side:

```bash
for f in 00001.dcm 00002.dcm 00003.dcm; do
  echo "$f"
  for e in StudyInstanceUID SeriesInstanceUID SOPInstanceUID InstanceNumber SliceLocation; do
    uvx pydicom show "$f::$e"
  done
done
```

```text
00001.dcm
2.25.3100000
2.25.3200000
2.25.3300001
1
-48
00002.dcm
2.25.3100000
2.25.3200000
2.25.3300002
2
-43
00003.dcm
2.25.3100000
2.25.3200000
2.25.3300003
3
-38
```

**Question.** Nothing outside these files says they belong together. How will a PACS know
that they are one series, and in which order to show them?

<details>
<summary>Answer</summary>

The three files carry the same Study Instance UID and the same Series Instance UID, and a
different SOP Instance UID each. That is all a PACS needs to file them as three instances of
one series. The order comes from inside the files too: the Instance Number, and the position
of each slice in the patient, here 5 mm apart (Slice Location −48, −43, −38).

</details>

## What you learned

- A DICOM file is a 128-byte preamble, `DICM`, the File Meta Information, then the data set.
- A data set is a list of elements in increasing order of tag. Each is a tag, a VR, a length
  and a value.
- The pixels are one element among the others, the last one, and by far the largest.
- A file names its own patient, study and series. Files with the same identifiers belong
  together.

Next: [Lab 2: store and search](lab-archive.md) hands these files to a PACS.
