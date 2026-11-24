# Conformance Statement

The DICOM Conformance Statement of pacs-rs's network services (PS3.2), written by the code
from what it negotiates (`cargo run -p pacs-dimse --example conformance`): a test fails when
this page and the code part. DICOMweb is described in [DICOMweb](dicomweb.md) and the
[reference](api-reference.md#dicomweb); what each service does, in [DIMSE](dimse.md).

## Implementation

| | |
|---|---|
| Implementation Class UID | `2.25.198281755599061360028760269505564383885`, in the files pacs-rs writes |
| Implementation Version Name | `PACS-RS 0.16.3` |
| On associations | the Implementation Class UID and Version Name of dicom-ul, the library pacs-rs negotiates with, which does not let them be set |
| Maximum PDU length | 32762 bytes received; what the peer says sent |
| Associations served at once | `PACS_DIMSE_MAX_ASSOCIATIONS`, 16 unless set; one more is rejected as transient, local limit exceeded |
| Asynchronous operations | none: one operation at a time on an association |

## Network services

| SOP Class | UID | SCU | SCP |
|---|---|---|---|
| Verification | `1.2.840.10008.1.1` | yes | yes |
| Study Root Query/Retrieve Information Model - FIND | `1.2.840.10008.5.1.4.1.2.2.1` | yes | yes |
| Patient Root Query/Retrieve Information Model - FIND | `1.2.840.10008.5.1.4.1.2.1.1` | yes | yes |
| Study Root Query/Retrieve Information Model - MOVE | `1.2.840.10008.5.1.4.1.2.2.2` | yes | yes |
| Patient Root Query/Retrieve Information Model - MOVE | `1.2.840.10008.5.1.4.1.2.1.2` | yes | yes |
| Study Root Query/Retrieve Information Model - GET | `1.2.840.10008.5.1.4.1.2.2.3` | yes | yes |
| Patient Root Query/Retrieve Information Model - GET | `1.2.840.10008.5.1.4.1.2.1.3` | yes | yes |
| The 173 Storage SOP Classes of PS3.4 Table B.5-1, [listed below](#storage-sop-classes) | | yes | yes |

## Application entities

One listener (`PACS_DIMSE_LISTEN`) answers pacs-rs's AE title (`PACS_DIMSE_AE_TITLE`, `PACSRS`
unless set), any further ones (`PACS_DIMSE_EXTRA_AE_TITLES`) and each project's; another called
AE title is rejected (PS3.8, reason 7). With `PACS_DIMSE_KNOWN_CALLERS_ONLY`, so is a calling AE
title not among the remote AEs (reason 3). Each AE title offers every service below unless
`PACS_DIMSE_CAPABILITIES` gives it its own services and transfer syntaxes; over TCP these are
what it negotiates, over TLS they hold per operation (status `0122`).

### Accepted presentation contexts

Every abstract syntax of the services, in the first transfer syntax the requester proposes
among these, whose data sets this build reads:

| Transfer syntax | UID |
|---|---|
| Implicit VR Little Endian | `1.2.840.10008.1.2` |
| Explicit VR Little Endian | `1.2.840.10008.1.2.1` |
| Encapsulated Uncompressed Explicit VR Little Endian | `1.2.840.10008.1.2.1.98` |
| Explicit VR Big Endian | `1.2.840.10008.1.2.2` |
| MPEG2 Main Profile / Main Level | `1.2.840.10008.1.2.4.100` |
| Fragmentable MPEG2 Main Profile / Main Level | `1.2.840.10008.1.2.4.100.1` |
| MPEG2 Main Profile / High Level | `1.2.840.10008.1.2.4.101` |
| Fragmentable MPEG2 Main Profile / High Level | `1.2.840.10008.1.2.4.101.1` |
| MPEG-4 AVC/H.264 High Profile / Level 4.1 | `1.2.840.10008.1.2.4.102` |
| Fragmentable MPEG-4 AVC/H.264 High Profile / Level 4.1 | `1.2.840.10008.1.2.4.102.1` |
| MPEG-4 AVC/H.264 BD-Compatible High Profile / Level 4.1 | `1.2.840.10008.1.2.4.103` |
| Fragmentable MPEG-4 AVC/H.264 BD-Compatible High Profile / Level 4.1 | `1.2.840.10008.1.2.4.103.1` |
| MPEG-4 AVC/H.264 High Profile / Level 4.2 For 2D Video | `1.2.840.10008.1.2.4.104` |
| Fragmentable MPEG-4 AVC/H.264 High Profile / Level 4.2 For 2D Video | `1.2.840.10008.1.2.4.104.1` |
| MPEG-4 AVC/H.264 High Profile / Level 4.2 For 3D Video | `1.2.840.10008.1.2.4.105` |
| Fragmentable MPEG-4 AVC/H.264 High Profile / Level 4.2 For 3D Video | `1.2.840.10008.1.2.4.105.1` |
| MPEG-4 AVC/H.264 Stereo High Profile / Level 4.2 | `1.2.840.10008.1.2.4.106` |
| Fragmentable MPEG-4 AVC/H.264 Stereo High Profile / Level 4.2 | `1.2.840.10008.1.2.4.106.1` |
| HEVC/H.265 Main Profile / Level 5.1 | `1.2.840.10008.1.2.4.107` |
| HEVC/H.265 Main 10 Profile / Level 5.1 | `1.2.840.10008.1.2.4.108` |
| JPEG XL Lossless | `1.2.840.10008.1.2.4.110` |
| JPEG XL Recompression | `1.2.840.10008.1.2.4.111` |
| JPEG XL | `1.2.840.10008.1.2.4.112` |
| High-Throughput JPEG 2000 Image Compression (Lossless Only) | `1.2.840.10008.1.2.4.201` |
| High-Throughput JPEG 2000 with RPCL Options Image Compression (Lossless Only) | `1.2.840.10008.1.2.4.202` |
| High-Throughput JPEG 2000 Image Compression | `1.2.840.10008.1.2.4.203` |
| JPIP HTJ2K Referenced | `1.2.840.10008.1.2.4.204` |
| JPEG Baseline (Process 1) | `1.2.840.10008.1.2.4.50` |
| JPEG Extended (Process 2 & 4) | `1.2.840.10008.1.2.4.51` |
| JPEG Lossless, Non-Hierarchical (Process 14) | `1.2.840.10008.1.2.4.57` |
| JPEG Lossless, Non-Hierarchical, First-Order Prediction | `1.2.840.10008.1.2.4.70` |
| JPEG-LS Lossless Image Compression | `1.2.840.10008.1.2.4.80` |
| JPEG-LS Lossy (Near-Lossless) Image Compression | `1.2.840.10008.1.2.4.81` |
| JPEG 2000 Image Compression (Lossless Only) | `1.2.840.10008.1.2.4.90` |
| JPEG 2000 Image Compression | `1.2.840.10008.1.2.4.91` |
| JPEG 2000 Part 2 Multi-component Image Compression (Lossless Only) | `1.2.840.10008.1.2.4.92` |
| JPEG 2000 Part 2 Multi-component Image Compression | `1.2.840.10008.1.2.4.93` |
| JPIP Referenced | `1.2.840.10008.1.2.4.94` |
| RLE Lossless | `1.2.840.10008.1.2.5` |
| SMPTE ST 2110-20 Uncompressed Progressive Active Video | `1.2.840.10008.1.2.7.1` |
| SMPTE ST 2110-20 Uncompressed Interlaced Active Video | `1.2.840.10008.1.2.7.2` |
| SMPTE ST 2110-30 PCM Digital Audio | `1.2.840.10008.1.2.7.3` |
| Deflated Image Frame Compression | `1.2.840.10008.1.2.8.1` |

Pixel data is decoded, to be transcoded or rendered, in Implicit and Explicit VR Little Endian,
JPEG-LS, JPEG XL and Deflated Image Frame Compression; in the others it is stored and sent as
received.

| Service | Role | Extended negotiation |
|---|---|---|
| Verification | SCP | none |
| Storage | SCP; SCU too for a C-GET's requester, whose SCP/SCU Role Selection (PS3.7 D.3.3.4) is accepted as proposed | none |
| FIND, MOVE, GET of both models | SCP | none: relational queries, combined date and time, fuzzy, empty value and multiple value matching are not negotiated |

### Proposed presentation contexts

| Operation | Abstract syntaxes | Transfer syntaxes | Role |
|---|---|---|---|
| C-ECHO | Verification | Implicit VR Little Endian | SCU |
| C-FIND | Study Root FIND, Patient Root FIND: Study Root used when accepted | Explicit VR Little Endian, Implicit VR Little Endian | SCU |
| C-MOVE | Study Root MOVE, Patient Root MOVE: Study Root used when accepted | Explicit VR Little Endian, Implicit VR Little Endian | SCU |
| C-GET | Study Root GET, Patient Root GET; and 126 Storage SOP Classes, the images most held first (27) | Explicit VR Little Endian, Implicit VR Little Endian for the GET models and most storage classes; the images also JPEG-LS Lossless Image Compression, JPEG Lossless, Non-Hierarchical, First-Order Prediction, JPEG Lossless, Non-Hierarchical (Process 14), JPEG 2000 Image Compression (Lossless Only), RLE Lossless, JPEG-LS Lossy (Near-Lossless) Image Compression, JPEG 2000 Image Compression, JPEG Baseline (Process 1) | SCU of GET, SCP of storage by role selection |
| C-STORE (sending, a retrieval's sub-operations) | the SOP class of each instance | the one it is stored in, and Explicit VR Little Endian, Implicit VR Little Endian; decoded to an uncompressed one when the peer takes only that | SCU |

At most 128 presentation contexts on an association (PS3.8 9.3.2.2).

## SOP specific conformance

### Storage (SCP)

Each data set is kept as received, with the file meta information added, then stored, compressed
losslessly when it came uncompressed (JPEG-LS, or deflated frames beyond 16 bits; `PACS_COMPRESSION`),
and indexed. Level 2 (Full) storage: every element is kept. An instance sent again is kept as
first stored unless `PACS_OVERWRITE_INSTANCES` is set, and answered Success either way.

| Status | Meaning |
|---|---|
| `0000` | stored, or already stored |
| `A700, A701` | Refused: Out of resources: the store, or the index, did not take it |
| `A702` | Refused: Out of resources: larger than `PACS_MAX_UPLOAD_BYTES` |
| `A900` | Data Set does not match SOP Class |
| `C000` | Cannot understand: the data set cannot be read, or lacks a UID |
| `C001` | SOP Instance UID is not the Affected SOP Instance UID |
| `C002` | the UIDs contradict the archive |
| `C003, C004` | a project refused or could not de-identify it |
| `0122` | SOP Class not supported, or not by the AE title called |

### Query (C-FIND SCP)

Hierarchical queries at every level of both models (PS3.4 C.6.1, C.6.2), matched by PS3.4
C.2.2.2: single value, wild card, list of UIDs, range, sequence; person names without regard to
case. Answered to the calling AE titles of `PACS_DIMSE_REMOTES`; another is refused `0124`.
The keys matched and returned:

| Model | Level | Keys |
|---|---|---|
| Patient Root | PATIENT | PatientName (PN), PatientID (LO), PatientBirthDate (DA), PatientSex (CS), NumberOfPatientRelatedStudies (IS), NumberOfPatientRelatedSeries (IS), NumberOfPatientRelatedInstances (IS) |
| Patient Root | STUDY | StudyDate (DA), StudyTime (TM), AccessionNumber (SH), ModalitiesInStudy (CS), ReferringPhysicianName (PN), StudyDescription (LO), StudyInstanceUID (UI), StudyID (SH), NumberOfStudyRelatedSeries (IS), NumberOfStudyRelatedInstances (IS) |
| Study Root | STUDY | StudyDate (DA), StudyTime (TM), AccessionNumber (SH), ModalitiesInStudy (CS), ReferringPhysicianName (PN), StudyDescription (LO), PatientName (PN), PatientID (LO), PatientBirthDate (DA), PatientSex (CS), StudyInstanceUID (UI), StudyID (SH), NumberOfStudyRelatedSeries (IS), NumberOfStudyRelatedInstances (IS) |
| both | SERIES | Modality (CS), SeriesDescription (LO), SeriesInstanceUID (UI), SeriesNumber (IS), NumberOfSeriesRelatedInstances (IS), PerformedProcedureStepStartDate (DA), PerformedProcedureStepStartTime (TM), RequestAttributesSequence (SQ) > ScheduledProcedureStepID, RequestedProcedureID |
| both | IMAGE | SOPClassUID (UI), SOPInstanceUID (UI), InstanceNumber (IS), NumberOfFrames (IS), Rows (US), Columns (US), BitsAllocated (US) |

### Retrieve (C-MOVE and C-GET SCP)

At every level of both models. A C-MOVE sends to the remote AEs of
`PACS_DIMSE_MOVE_DESTINATIONS` only, refusing any other with `A801`; a C-GET sends on the
association that asks. Sub-operations are counted in Pending responses; a C-CANCEL stops
before the next instance. Final statuses: `0000`, `B000` (some failed or warned), `FE00`
(cancelled), `A702` (none could be sent), `A801`, `A900`, `0124` (a caller not answered).

## Communication profiles

TCP/IP (PS3.8). With `PACS_DIMSE_TLS_CERT_FILE` and `PACS_DIMSE_TLS_KEY_FILE` the listener
speaks TLS only, by the BCP 195 profile of PS3.15 B.12, and asks callers for a certificate
unless told not to; remote AEs marked `/tls` are called over TLS.

## Security

Every association, search, retrieval and storage is written to the audit trail (PS3.15 A.5)
when `PACS_AUDIT_FILE` is set. Which calling AE titles are answered is configured (above).

## Character sets

Data sets are kept in the Specific Character Set they came in. Identifiers pacs-rs writes are
in ISO_IR 192 (UTF-8) when a value is not ASCII.

## Storage SOP Classes

| SOP Class | UID |
|---|---|
| Computed Radiography Image Storage | `1.2.840.10008.5.1.4.1.1.1` |
| Digital X-Ray Image Storage - For Presentation | `1.2.840.10008.5.1.4.1.1.1.1` |
| Digital X-Ray Image Storage - For Processing | `1.2.840.10008.5.1.4.1.1.1.1.1` |
| Digital Mammography X-Ray Image Storage - For Presentation | `1.2.840.10008.5.1.4.1.1.1.2` |
| Digital Mammography X-Ray Image Storage - For Processing | `1.2.840.10008.5.1.4.1.1.1.2.1` |
| Digital Intra-Oral X-Ray Image Storage - For Presentation | `1.2.840.10008.5.1.4.1.1.1.3` |
| Digital Intra-Oral X-Ray Image Storage - For Processing | `1.2.840.10008.5.1.4.1.1.1.3.1` |
| CT Image Storage | `1.2.840.10008.5.1.4.1.1.2` |
| CT Image Storage - For Processing | `1.2.840.10008.5.1.4.1.1.2.3` |
| Enhanced CT Image Storage | `1.2.840.10008.5.1.4.1.1.2.1` |
| Enhanced CT Image Storage - For Processing | `1.2.840.10008.5.1.4.1.1.2.4` |
| Legacy Converted Enhanced CT Image Storage | `1.2.840.10008.5.1.4.1.1.2.2` |
| Legacy Converted Enhanced CT Image Storage - For Processing | `1.2.840.10008.5.1.4.1.1.2.5` |
| Ultrasound Multi-frame Image Storage | `1.2.840.10008.5.1.4.1.1.3.1` |
| MR Image Storage | `1.2.840.10008.5.1.4.1.1.4` |
| Enhanced MR Image Storage | `1.2.840.10008.5.1.4.1.1.4.1` |
| MR Spectroscopy Storage | `1.2.840.10008.5.1.4.1.1.4.2` |
| Enhanced MR Color Image Storage | `1.2.840.10008.5.1.4.1.1.4.3` |
| Legacy Converted Enhanced MR Image Storage | `1.2.840.10008.5.1.4.1.1.4.4` |
| Ultrasound Image Storage | `1.2.840.10008.5.1.4.1.1.6.1` |
| Enhanced US Volume Storage | `1.2.840.10008.5.1.4.1.1.6.2` |
| Photoacoustic Image Storage | `1.2.840.10008.5.1.4.1.1.6.3` |
| Secondary Capture Image Storage | `1.2.840.10008.5.1.4.1.1.7` |
| Multi-frame Single Bit Secondary Capture Image Storage | `1.2.840.10008.5.1.4.1.1.7.1` |
| Multi-frame Grayscale Byte Secondary Capture Image Storage | `1.2.840.10008.5.1.4.1.1.7.2` |
| Multi-frame Grayscale Word Secondary Capture Image Storage | `1.2.840.10008.5.1.4.1.1.7.3` |
| Multi-frame True Color Secondary Capture Image Storage | `1.2.840.10008.5.1.4.1.1.7.4` |
| 12-lead ECG Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.1.1` |
| General ECG Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.1.2` |
| Ambulatory ECG Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.1.3` |
| General 32-bit ECG Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.1.4` |
| Hemodynamic Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.2.1` |
| Cardiac Electrophysiology Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.3.1` |
| Basic Voice Audio Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.4.1` |
| General Audio Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.4.2` |
| Arterial Pulse Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.5.1` |
| Respiratory Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.6.1` |
| Multi-channel Respiratory Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.6.2` |
| Routine Scalp Electroencephalogram Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.7.1` |
| Electromyogram Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.7.2` |
| Electrooculogram Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.7.3` |
| Sleep Electroencephalogram Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.7.4` |
| Body Position Waveform Storage | `1.2.840.10008.5.1.4.1.1.9.8.1` |
| Waveform Presentation State Storage | `1.2.840.10008.5.1.4.1.1.9.100.1` |
| Waveform Acquisition Presentation State Storage | `1.2.840.10008.5.1.4.1.1.9.100.2` |
| Grayscale Softcopy Presentation State Storage | `1.2.840.10008.5.1.4.1.1.11.1` |
| Color Softcopy Presentation State Storage | `1.2.840.10008.5.1.4.1.1.11.2` |
| Pseudo-Color Softcopy Presentation State Storage | `1.2.840.10008.5.1.4.1.1.11.3` |
| Blending Softcopy Presentation State Storage | `1.2.840.10008.5.1.4.1.1.11.4` |
| XA/XRF Grayscale Softcopy Presentation State Storage | `1.2.840.10008.5.1.4.1.1.11.5` |
| Grayscale Planar MPR Volumetric Presentation State Storage | `1.2.840.10008.5.1.4.1.1.11.6` |
| Compositing Planar MPR Volumetric Presentation State Storage | `1.2.840.10008.5.1.4.1.1.11.7` |
| Advanced Blending Presentation State Storage | `1.2.840.10008.5.1.4.1.1.11.8` |
| Volume Rendering Volumetric Presentation State Storage | `1.2.840.10008.5.1.4.1.1.11.9` |
| Segmented Volume Rendering Volumetric Presentation State Storage | `1.2.840.10008.5.1.4.1.1.11.10` |
| Multiple Volume Rendering Volumetric Presentation State Storage | `1.2.840.10008.5.1.4.1.1.11.11` |
| Variable Modality LUT Softcopy Presentation State Storage | `1.2.840.10008.5.1.4.1.1.11.12` |
| X-Ray Angiographic Image Storage | `1.2.840.10008.5.1.4.1.1.12.1` |
| Enhanced XA Image Storage | `1.2.840.10008.5.1.4.1.1.12.1.1` |
| X-Ray Radiofluoroscopic Image Storage | `1.2.840.10008.5.1.4.1.1.12.2` |
| Enhanced XRF Image Storage | `1.2.840.10008.5.1.4.1.1.12.2.1` |
| X-Ray 3D Angiographic Image Storage | `1.2.840.10008.5.1.4.1.1.13.1.1` |
| X-Ray 3D Craniofacial Image Storage | `1.2.840.10008.5.1.4.1.1.13.1.2` |
| Breast Tomosynthesis Image Storage | `1.2.840.10008.5.1.4.1.1.13.1.3` |
| Breast Projection X-Ray Image Storage - For Presentation | `1.2.840.10008.5.1.4.1.1.13.1.4` |
| Breast Projection X-Ray Image Storage - For Processing | `1.2.840.10008.5.1.4.1.1.13.1.5` |
| Intravascular Optical Coherence Tomography Image Storage - For Presentation | `1.2.840.10008.5.1.4.1.1.14.1` |
| Intravascular Optical Coherence Tomography Image Storage - For Processing | `1.2.840.10008.5.1.4.1.1.14.2` |
| Nuclear Medicine Image Storage | `1.2.840.10008.5.1.4.1.1.20` |
| Parametric Map Storage | `1.2.840.10008.5.1.4.1.1.30` |
| Raw Data Storage | `1.2.840.10008.5.1.4.1.1.66` |
| Spatial Registration Storage | `1.2.840.10008.5.1.4.1.1.66.1` |
| Spatial Fiducials Storage | `1.2.840.10008.5.1.4.1.1.66.2` |
| Deformable Spatial Registration Storage | `1.2.840.10008.5.1.4.1.1.66.3` |
| Segmentation Storage | `1.2.840.10008.5.1.4.1.1.66.4` |
| Surface Segmentation Storage | `1.2.840.10008.5.1.4.1.1.66.5` |
| Tractography Results Storage | `1.2.840.10008.5.1.4.1.1.66.6` |
| Label Map Segmentation Storage | `1.2.840.10008.5.1.4.1.1.66.7` |
| Height Map Segmentation Storage | `1.2.840.10008.5.1.4.1.1.66.8` |
| Real World Value Mapping Storage | `1.2.840.10008.5.1.4.1.1.67` |
| Surface Scan Mesh Storage | `1.2.840.10008.5.1.4.1.1.68.1` |
| Surface Scan Point Cloud Storage | `1.2.840.10008.5.1.4.1.1.68.2` |
| VL Endoscopic Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.1` |
| Video Endoscopic Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.1.1` |
| VL Microscopic Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.2` |
| Video Microscopic Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.2.1` |
| VL Slide-Coordinates Microscopic Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.3` |
| VL Photographic Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.4` |
| Video Photographic Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.4.1` |
| Ophthalmic Photography 8 Bit Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.5.1` |
| Ophthalmic Photography 16 Bit Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.5.2` |
| Stereometric Relationship Storage | `1.2.840.10008.5.1.4.1.1.77.1.5.3` |
| Ophthalmic Tomography Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.5.4` |
| Wide Field Ophthalmic Photography Stereographic Projection Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.5.5` |
| Wide Field Ophthalmic Photography 3D Coordinates Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.5.6` |
| Ophthalmic Optical Coherence Tomography En Face Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.5.7` |
| Ophthalmic Optical Coherence Tomography B-scan Volume Analysis Storage | `1.2.840.10008.5.1.4.1.1.77.1.5.8` |
| VL Whole Slide Microscopy Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.6` |
| Dermoscopic Photography Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.7` |
| Confocal Microscopy Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.8` |
| Confocal Microscopy Tiled Pyramidal Image Storage | `1.2.840.10008.5.1.4.1.1.77.1.9` |
| Lensometry Measurements Storage | `1.2.840.10008.5.1.4.1.1.78.1` |
| Autorefraction Measurements Storage | `1.2.840.10008.5.1.4.1.1.78.2` |
| Keratometry Measurements Storage | `1.2.840.10008.5.1.4.1.1.78.3` |
| Subjective Refraction Measurements Storage | `1.2.840.10008.5.1.4.1.1.78.4` |
| Visual Acuity Measurements Storage | `1.2.840.10008.5.1.4.1.1.78.5` |
| Spectacle Prescription Report Storage | `1.2.840.10008.5.1.4.1.1.78.6` |
| Ophthalmic Axial Measurements Storage | `1.2.840.10008.5.1.4.1.1.78.7` |
| Intraocular Lens Calculations Storage | `1.2.840.10008.5.1.4.1.1.78.8` |
| Macular Grid Thickness and Volume Report | `1.2.840.10008.5.1.4.1.1.79.1` |
| Ophthalmic Visual Field Static Perimetry Measurements Storage | `1.2.840.10008.5.1.4.1.1.80.1` |
| Ophthalmic Thickness Map Storage | `1.2.840.10008.5.1.4.1.1.81.1` |
| Corneal Topography Map Storage | `1.2.840.10008.5.1.4.1.1.82.1` |
| Basic Text SR Storage | `1.2.840.10008.5.1.4.1.1.88.11` |
| Enhanced SR Storage | `1.2.840.10008.5.1.4.1.1.88.22` |
| Comprehensive SR Storage | `1.2.840.10008.5.1.4.1.1.88.33` |
| Comprehensive 3D SR Storage | `1.2.840.10008.5.1.4.1.1.88.34` |
| Extensible SR Storage | `1.2.840.10008.5.1.4.1.1.88.35` |
| Procedure Log Storage | `1.2.840.10008.5.1.4.1.1.88.40` |
| Mammography CAD SR Storage | `1.2.840.10008.5.1.4.1.1.88.50` |
| Key Object Selection Document Storage | `1.2.840.10008.5.1.4.1.1.88.59` |
| Chest CAD SR Storage | `1.2.840.10008.5.1.4.1.1.88.65` |
| X-Ray Radiation Dose SR Storage | `1.2.840.10008.5.1.4.1.1.88.67` |
| Radiopharmaceutical Radiation Dose SR Storage | `1.2.840.10008.5.1.4.1.1.88.68` |
| Colon CAD SR Storage | `1.2.840.10008.5.1.4.1.1.88.69` |
| Implantation Plan SR Storage | `1.2.840.10008.5.1.4.1.1.88.70` |
| Acquisition Context SR Storage | `1.2.840.10008.5.1.4.1.1.88.71` |
| Simplified Adult Echo SR Storage | `1.2.840.10008.5.1.4.1.1.88.72` |
| Patient Radiation Dose SR Storage | `1.2.840.10008.5.1.4.1.1.88.73` |
| Planned Imaging Agent Administration SR Storage | `1.2.840.10008.5.1.4.1.1.88.74` |
| Performed Imaging Agent Administration SR Storage | `1.2.840.10008.5.1.4.1.1.88.75` |
| Enhanced X-Ray Radiation Dose SR Storage | `1.2.840.10008.5.1.4.1.1.88.76` |
| Waveform Annotation SR Storage | `1.2.840.10008.5.1.4.1.1.88.77` |
| Content Assessment Results Storage | `1.2.840.10008.5.1.4.1.1.90.1` |
| Microscopy Bulk Simple Annotations Storage | `1.2.840.10008.5.1.4.1.1.91.1` |
| Encapsulated PDF Storage | `1.2.840.10008.5.1.4.1.1.104.1` |
| Encapsulated CDA Storage | `1.2.840.10008.5.1.4.1.1.104.2` |
| Encapsulated STL Storage | `1.2.840.10008.5.1.4.1.1.104.3` |
| Encapsulated OBJ Storage | `1.2.840.10008.5.1.4.1.1.104.4` |
| Encapsulated MTL Storage | `1.2.840.10008.5.1.4.1.1.104.5` |
| Positron Emission Tomography Image Storage | `1.2.840.10008.5.1.4.1.1.128` |
| Enhanced PET Image Storage | `1.2.840.10008.5.1.4.1.1.130` |
| Legacy Converted Enhanced PET Image Storage | `1.2.840.10008.5.1.4.1.1.128.1` |
| Basic Structured Display Storage | `1.2.840.10008.5.1.4.1.1.131` |
| CT Performed Procedure Protocol Storage | `1.2.840.10008.5.1.4.1.1.200.2` |
| XA Performed Procedure Protocol Storage | `1.2.840.10008.5.1.4.1.1.200.8` |
| RT Image Storage | `1.2.840.10008.5.1.4.1.1.481.1` |
| RT Dose Storage | `1.2.840.10008.5.1.4.1.1.481.2` |
| RT Structure Set Storage | `1.2.840.10008.5.1.4.1.1.481.3` |
| RT Beams Treatment Record Storage | `1.2.840.10008.5.1.4.1.1.481.4` |
| RT Plan Storage | `1.2.840.10008.5.1.4.1.1.481.5` |
| RT Brachy Treatment Record Storage | `1.2.840.10008.5.1.4.1.1.481.6` |
| RT Treatment Summary Record Storage | `1.2.840.10008.5.1.4.1.1.481.7` |
| RT Ion Plan Storage | `1.2.840.10008.5.1.4.1.1.481.8` |
| RT Ion Beams Treatment Record Storage | `1.2.840.10008.5.1.4.1.1.481.9` |
| RT Physician Intent Storage | `1.2.840.10008.5.1.4.1.1.481.10` |
| RT Segment Annotation Storage | `1.2.840.10008.5.1.4.1.1.481.11` |
| RT Radiation Set Storage | `1.2.840.10008.5.1.4.1.1.481.12` |
| C-Arm Photon-Electron Radiation Storage | `1.2.840.10008.5.1.4.1.1.481.13` |
| Tomotherapeutic Radiation Storage | `1.2.840.10008.5.1.4.1.1.481.14` |
| Robotic-Arm Radiation Storage | `1.2.840.10008.5.1.4.1.1.481.15` |
| RT Radiation Record Set Storage | `1.2.840.10008.5.1.4.1.1.481.16` |
| RT Radiation Salvage Record Storage | `1.2.840.10008.5.1.4.1.1.481.17` |
| Tomotherapeutic Radiation Record Storage | `1.2.840.10008.5.1.4.1.1.481.18` |
| C-Arm Photon-Electron Radiation Record Storage | `1.2.840.10008.5.1.4.1.1.481.19` |
| Robotic Radiation Record Storage | `1.2.840.10008.5.1.4.1.1.481.20` |
| RT Radiation Set Delivery Instruction Storage | `1.2.840.10008.5.1.4.1.1.481.21` |
| RT Treatment Preparation Storage | `1.2.840.10008.5.1.4.1.1.481.22` |
| Enhanced RT Image Storage | `1.2.840.10008.5.1.4.1.1.481.23` |
| Enhanced Continuous RT Image Storage | `1.2.840.10008.5.1.4.1.1.481.24` |
| RT Patient Position Acquisition Instruction Storage | `1.2.840.10008.5.1.4.1.1.481.25` |
| RT Beams Delivery Instruction Storage | `1.2.840.10008.5.1.4.34.7` |
| RT Brachy Application Setup Delivery Instruction Storage | `1.2.840.10008.5.1.4.34.10` |
