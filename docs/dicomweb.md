# DICOMweb

Served under `/dicom-web` (`PACS_DICOMWEB_PATH`, see [Settings](settings.md#addresses)). Each request,
with its parameters and statuses, is in the [reference](api-reference.md#dicomweb).

| Request | |
|---|---|
| `GET /studies`, `/studies/{study}/series`, `.../series/{series}/instances` | QIDO-RS; counts, Modalities in Study and Retrieve URL always included |
| `GET /studies/{study}`, `.../series/{series}`, `.../instances/{instance}` | WADO-RS retrieve, `multipart/related; type="application/dicom"`, as stored or transcoded (below) |
| `GET .../metadata` (study, series, instance) | DICOM JSON without pixel data |
| `GET .../instances/{instance}/frames/{list}` | frames as stored or transcoded: `application/octet-stream` or the compressed type of PS3.18 Table 8.7.3-5 |
| `GET .../instances/{instance}/rendered`, `.../frames/{frame}/rendered` | a frame as a picture, JPEG or PNG ([below](#rendered)) |
| `GET .../instances/{instance}/bulkdata/{path}` | a value metadata names by a BulkDataURI ([below](#bulk-data)) |
| `POST /studies`, `POST /studies/{study}` | STOW-RS, `application/dicom` parts |

Not yet: fuzzy, empty value and multiple value matching ([below](#searching)); JSON/XML +
bulk data STOW; rendered series and studies, thumbnails.

## Searching

QIDO-RS matches as C-FIND does (PS3.18 8.3.4.1, PS3.4 C.2.2.2), on the required keys of
PS3.18 Table 10.6.1-5 and a few more the index keeps.

| A value | Matches | For |
|---|---|---|
| `LITS-0` | that value: with regard to case, except for names | every key |
| `DOE*`, `D?E^*` | wild cards: `*` any run of characters, `?` any one | text and names |
| `20260101-20260131`, `20260101-`, `-20260131` | a range, both ends included | dates and times |
| `1.2.3,1.2.4` | any of the UIDs | UIDs |
| empty, or `*` alone | everything | every key |

| Level | Keys |
|---|---|
| Studies | Study Date, Study Time, Accession Number, Modalities In Study, Referring Physician Name, Patient Name, Patient ID, Study Instance UID, Study ID; and Study Description, Patient Birth Date, Patient Sex |
| A study's series | Modality, Series Instance UID, Series Number, Performed Procedure Step Start Date and Time, Request Attributes Sequence (its Scheduled Procedure Step ID and Requested Procedure ID); and Series Description |
| A series' instances | SOP Class UID, SOP Instance UID, Instance Number |

- **Names** are matched without regard to case, as the standard lets a server choose
  (C.2.2.2.1.1); everything else is case-sensitive. Only the alphabetic group of a name is
  matched.
- **A time is matched by its meaning**: `1030` is every time in that minute, and `0900-1100`
  ends at 11:00:59.999999.
- **A range of dates with a range of times is one range of moments** (PS3.18 8.3.4.1.1):
  `StudyDate=20260105-20260107&StudyTime=1100-1000` is from the 5th at 11:00 to the 7th at
  10:00. A range of times that ends before it starts is refused on its own.
- **Modalities In Study** matches a study when any of its series does, and an attribute
  with several values when any of them does (PS3.4 C.2.2.3).
- A key given twice, one this level does not have, or a value that is not a match for its
  key (a date that is not `YYYYMMDD`, a wild card in a UID) is answered 400 with the reason,
  never by matching something else.
- `fuzzymatching=true`, `emptyvaluematching=true` and `multiplevaluematching=true` are not
  done: the search is made without, and the answer carries the `Warning: 299` header the
  standard gives for each (PS3.18 8.3.4.2, 8.3.4.5, 8.3.4.6).
- **A key of a sequence's item** is named `Sequence.Attribute`, by keyword or tag
  (PS3.18 8.3.4.1): `RequestAttributesSequence.ScheduledProcedureStepID=STEP1`, or
  `00400275.00400009=STEP1`. A series answers when one item of its sequence answers every
  key asked of that sequence (PS3.4 C.2.2.2.6, as [C-FIND](dimse.md#being-searched)
  matches): `…ScheduledProcedureStepID=STEP1&RequestAttributesSequence.RequestedProcedureID=REQ2`
  finds nothing when STEP1 and REQ2 are in two items of the series. The sequence itself takes
  no value.
- **`includefield`** adds attributes to each answer, by keyword or tag, comma-separated or
  repeated (PS3.18 8.3.4.3): `includefield=Manufacturer,00180050`. They come from the
  metadata the index keeps of every instance: an instance answers with its own, a study or a
  series with those of its first instance by Instance Number. `includefield=all` gives the
  modules of the level: at the study level the Patient, General Study and Patient Study
  Modules, at the series level those and the General Series, General Equipment and Frame of
  Reference Modules, at the instance level the whole data set without its pixel data.
  Nothing an answer already has is changed; a name that is no attribute is 400.
- Not a key yet: the searches from the root for all series or all instances.

SQLite and Postgres answer the same: one rule in the code decides every match, and the
database only narrows what it is asked about.

## Bulk data

In metadata, a binary value of more than 1 KiB is not written inline (`InlineBinary`) but named
by a `BulkDataURI` (PS3.18 8.6.1.3, Annex F.2.6): `.../instances/{instance}/bulkdata/{path}`, the
path being the element's tag, or the tags and item numbers from 0 down to it inside sequences
(`00400275/0/00081032`). That address answers the value's bytes, `multipart/related;
type="application/octet-stream"`, or alone for `Accept: application/octet-stream`. Smaller values
stay inline. The pixel data is not in metadata, and is retrieved by its frames.

## Rendered

A frame as a picture for clients that do not read DICOM, a web page's `<img>` or a report
(PS3.18 9.5): an instance's first frame, or one frame by its number. JPEG, which every server
makes and which is given without an `Accept` header or for `image/*`, or PNG when the client
prefers `image/png`; anything else is 406.

- **Monochrome** goes through its rescale (Rescale Slope and Intercept, PS3.3 C.11.1), then a
  window (C.11.2) to 8 bits: the one asked for, `window=center,width,function` with the
  function `linear`, `linear-exact` or `sigmoid`; else the file's first Window Center and
  Width with its VOI LUT Function; else the frame's own range of values. MONOCHROME1 is
  inverted, so white is high either way.
- **Colour**: RGB as it is, YBR_FULL made RGB, palette colour through its lookup tables; 16-bit
  samples are brought to 8. Subsampled colour (YBR_FULL_422) and segmented palettes are not
  rendered (422).
- **`viewport=vw,vh`** fits the picture in that box, its proportions kept;
  `viewport=vw,vh,sx,sy,sw,sh` renders only that region of the frame. **`quality`** is the
  JPEG's, 1 to 100 (90). `annotation` is taken and not done: nothing is written on the
  picture.

`GET /dicom-web/studies/{study}/series/{series}/instances/{instance}/rendered?window=40,400,linear&viewport=256,256`
is a 256 x 256 JPEG of a CT slice in an abdominal window.

## Compression and transcoding

Uncompressed pixel data is stored as **JPEG-LS lossless** (`1.2.840.10008.1.2.4.80`, codecsls):
each frame is decoded back and compared with what arrived before the file is kept, and an
instance that does not come back exact, or that the codecs do not handle (subsampled colour
such as YBR_FULL_422), is stored as received. Pixel data of more than 16 bits, which JPEG-LS
does not code (an RT Dose of 32-bit values), is stored with each frame deflated instead:
**Deflated Image Frame Compression** (`1.2.840.10008.1.2.8.1`, PS3.5 10.20), lossless, a
fragment a frame, checked the same way. So is anything that arrives compressed, **JPEG XL** included.
`PACS_COMPRESSION=off` stores everything as received.

The client's preference decides what a retrieve sends (PS3.18 8.7.3.5): the Accept header's
ranges by quality, those of equal quality in the order sent. Within the first range that takes
anything, the content as stored wins (a viewer asking for `transfer-syntax=*` gets the stored
JPEG-LS untouched), then these, in this order; nothing else can be made, so anything else is 406.
So one header asks for JPEG-LS where the server can make it and anything otherwise:
`multipart/related; type="image/jls", multipart/related; type="application/octet-stream"; transfer-syntax=*`.

| Transfer syntax | Frames | Decoded by | Encoded by |
|---|---|---|---|
| Explicit VR Little Endian `1.2.840.10008.1.2.1` | `application/octet-stream` | - | - |
| JPEG-LS lossless `1.2.840.10008.1.2.4.80` (`.81` read) | `image/jls` | codecsls | codecsls |
| JPEG XL lossless `1.2.840.10008.1.2.4.110` (`.112` read) | `image/jxl` | jxl-rs | zune-jpegxl |
| Deflated Image Frame Compression `1.2.840.10008.1.2.8.1` | `application/octet-stream` | flate2 | flate2 |

Deflated frames are octet streams few clients inflate, so they are given only to a client that
names their transfer syntax (`transfer-syntax=1.2.840.10008.1.2.8.1`); an octet stream of no
transfer syntax (Explicit VR Little Endian, PS3.18 Table 8.7.3-5), `transfer-syntax=*` and
`*/*` get the frames inflated, and a file asked for without a transfer syntax comes native.

Monochrome, palette colour, RGB and YBR_FULL, 8 or 16 bits allocated (32 for monochrome in
deflated frames); signed pixels are coded
as their raw bit pattern, as DCMTK and GDCM do. Colour is coded by pixel, and a file that had
its samples by plane is kept with Planar Configuration 0, as compressed colour always is (PS3.5
8.2.3); decoded again, it comes back by pixel. Palette colour keeps its lookup tables as they
are. JPEG XL is for monochrome only here.

JPEG XL is served and accepted, not chosen for storage. On 172 slices of a public CT (3D-IRCADb
patient 1.2, signed 16-bit, one core) JPEG-LS took the pixels to 1/3.38 at about 5 ms a slice
with CharLS, which pacs-rs used then (codecsls writes the same bytes, at about 2.5 ms a slice);
zune-jpegxl, the pure-Rust encoder, reached 1/2.82, and libjxl at effort 7 1/3.37 at about
100 ms a slice. Through the server (with CharLS) the study went from 90.4 MB to 26.8 MB, ingested in 3.3 s,
and every frame came back exact as uncompressed, JPEG-LS and JPEG XL (checked with CharLS and
libjxl, not only with the codecs above); jxl-rs decoded libjxl's own effort-7 output of the same
slices exactly.

## Elements in UN, wrong lengths and costly files

A file re-encoded from Implicit VR may carry standard elements as UN, which PS3.5 6.2.2 allows: an
undefined-length UN holds a sequence in Implicit VR Little Endian whatever the transfer syntax,
and any other UN is little endian. Every read goes through `pacs_core::wire::checked`, which gives
dicom-rs such elements under their dictionary VR, as DCMTK and dcm4che read them. The stored file
stays as received (a native one is written anew when compressed, then under the real VRs). UN stays
UN for private elements with a defined length, for tags the dictionary does not have, and for
values too long for the VR's 16-bit length or of a size it does not come in.

A value declared longer than the file, or nesting deeper than 64 sequences, is refused before
dicom-rs allocates for it: 400 on `POST /instances`, a failed part in STOW-RS, a failure status in
C-STORE. A last fragment of encapsulated pixel data cut short stays readable, as it was.

Some well-formed files take far more memory to read than their size: an 8 MB file of one-byte text
values takes 1.3 GB with its metadata JSON, while pixel data takes its own size
([benchmarks](benchmarks.md)). The same walk counts elements, items and text values into an estimate. A
file estimated over `PACS_MAX_READ_BYTES` is refused unread (413 on `POST /instances`, failure
reason A700 in STOW-RS, status A702 in C-STORE), and files being read share that amount, so that
some at once wait their turn instead of adding up. It is an estimate for one file at its worst:
allow about 1.6 times it, so keep it near half of the memory the server has. The default lets through about
3 million text values or 600 000 elements; raise it for larger structure sets or enhanced
multi-frame objects if the memory is there. Bodies are not counted, since they are in memory before
the walk: `PACS_MAX_UPLOAD_BYTES` and concurrent clients bound those. Identifiers in C-FIND and
C-MOVE responses go through the same walk, with a fixed limit of 64 MiB.

## Browser caching

Retrieves, frames and metadata carry a weak `ETag` and `Vary: Accept`; a request whose
`If-None-Match` holds the tag gets 304 Not Modified before any file is read. The tag is a hash of
the instances' storage keys, sizes and receipt times, the Accept header and the server's version
(metadata: of the body). One instance and its frames cannot change while instances are never
overwritten, so they are `private, max-age=86400`: a study opened again within a day costs no
request at all. Studies, series (they gain instances) and metadata, and everything when
`PACS_OVERWRITE_INSTANCES` is set, are `private, no-cache`: asked about each time, at the cost of
a round trip, not the bytes. `private` keeps them out of shared caches, since the deployment in
front decides who may read a study; a browser keeps for up to a day what it has already read.

On a 160-slice 512x512 CT stored uncompressed, one open with the header above: 83.9 MB as
stored from v0.4.0, 31.6 MB of JPEG-LS from v0.5.0, and opened again with the tags, 160 answers
of 304 and no pixel bytes.

DICOM JSON numbers: IS and DS values are written as JSON numbers, padding trimmed (PS3.18 Table
F.2.3-1 allows numbers or strings).
