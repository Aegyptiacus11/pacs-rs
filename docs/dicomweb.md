# DICOMweb

Served under `/dicom-web` (`PACS_DICOMWEB_PATH`, see [Running](running.md)).

| Request | |
|---|---|
| `GET /studies`, `/studies/{study}/series`, `.../series/{series}/instances` | QIDO-RS; counts, Modalities in Study and Retrieve URL always included |
| `GET /studies/{study}`, `.../series/{series}`, `.../instances/{instance}` | WADO-RS retrieve, `multipart/related; type="application/dicom"`, as stored or transcoded (below) |
| `GET .../metadata` (study, series, instance) | DICOM JSON without pixel data |
| `GET .../instances/{instance}/frames/{list}` | frames as stored or transcoded: `application/octet-stream` or the compressed type of PS3.18 Table 8.7.3-5 |
| `POST /studies`, `POST /studies/{study}` | STOW-RS, `application/dicom` parts |

Not yet: wildcard, range, list and fuzzy matching (answered with 400, not ignored);
`includefield` for attributes beyond the stored levels; rendered media; JSON/XML + bulk data
STOW; BulkDataURI in metadata.

## Compression and transcoding

Uncompressed pixel data is stored as **JPEG-LS lossless** (`1.2.840.10008.1.2.4.80`, CharLS):
each frame is decoded back and compared with what arrived before the file is kept, and an
instance that does not come back exact, or that the codecs do not handle (colour, more than 16
bits), is stored as received. So is anything that arrives compressed, **JPEG XL** included.
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
| JPEG-LS lossless `1.2.840.10008.1.2.4.80` (`.81` read) | `image/jls` | CharLS | CharLS |
| JPEG XL lossless `1.2.840.10008.1.2.4.110` (`.112` read) | `image/jxl` | jxl-rs | zune-jpegxl |

Grayscale, 8 or 16 bits allocated; signed pixels are coded as their raw bit pattern, as DCMTK and
GDCM do.

JPEG XL is served and accepted, not chosen for storage. On 172 slices of a public CT (3D-IRCADb
patient 1.2, signed 16-bit, one core) JPEG-LS took the pixels to 1/3.38 at about 5 ms a slice;
zune-jpegxl, the pure-Rust encoder, reached 1/2.82, and libjxl at effort 7 1/3.37 at about
100 ms a slice. Through the server the study went from 90.4 MB to 26.8 MB, ingested in 3.3 s,
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
