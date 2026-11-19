# Test files from pydicom

DICOM files taken unchanged from the test data of [pydicom](https://github.com/pydicom/pydicom)
3.0.2, for the tests that store, search and send real DICOM (`pacs_core::real`). Five are in
pydicom's package (`pydicom/data/test_files`); the two `emri_small` files are in
[pydicom-data](https://github.com/pydicom/pydicom-data) at commit `39a2eb3`, which pydicom
downloads when a test asks for them, and were checked against the hashes pydicom keeps for
them (`pydicom/data/hashes.json`).

| File | What it is | SHA-256 |
|---|---|---|
| `CT_small.dcm` | CT Image Storage, GE Medical Systems, Explicit VR Little Endian, 128 x 128, 16 bits | `3dd31e5cc835b3f2cdd46c9da1982f59251e78518fefa8163d914631c66437d6` |
| `MR_small.dcm` | MR Image Storage, Toshiba, Explicit VR Little Endian, 64 x 64, 16 bits | `3f27d1c22f1a66e80d7bb7c911e8610fd0bb70325a76746a7adb1c0ddefcf2bb` |
| `MR_small_jpeg_ls_lossless.dcm` | `MR_small.dcm` in JPEG-LS Lossless, one fragment | `b2b69dd2ae854bf7dfada6745709cd5d8a4573ea12387adbbdc56e8be6056206` |
| `JPEG-lossy.dcm` | a nuclear medicine image, GE Medical Systems, JPEG Extended (Process 2 and 4), 1024 x 256 | `c425608e2fcda8332c75d33f890bfe3bae32700608b719046b3d9e789374c292` |
| `emri_small.dcm` | Enhanced MR Image Storage, ten frames of 64 x 64, Explicit VR Little Endian | `151233ec63f64ebb63b979df51aa827cd612a53422c073f6ef341770c7bc9a56` |
| `emri_small_jpeg_ls_lossless.dcm` | `emri_small.dcm` in JPEG-LS Lossless, a fragment a frame, an empty offset table | `24de03c9c0f8b5aa75d7fbcc894f94e612b66702175b4936589a0849ec9f87b4` |
| `UN_sequence.dcm` | a data set of one private sequence written in UN, as GDCM 2.4.4 left it | `09f033e0d40a18c4ade0b08b8d318ab6deb684a84f503d4ec4f00908883dd431` |

## Where they come from

As pydicom's own `README.txt` beside its test files says:

- `CT_small.dcm` was downsized to 128 x 128 from `CT1_UNC`, and `MR_small.dcm` to 64 x 64 from
  `MR1_UNC`, both of the NEMA WG04 compression sample set
  (`ftp://medical.nema.org/MEDICAL/Dicom/DataSets/WG04/`). The patient names in them are the
  sample set's (`CompressedSamples^CT1`, `CompressedSamples^MR1`).
- `JPEG-lossy.dcm` is `NM1_JPLY` of the same sample set.
- `UN_sequence.dcm` was given to pydicom by a user to reproduce its issue 1312.

Not said there, so not known here: who made `emri_small.dcm` and from what. It names no
manufacturer and its dates are 2000-01-01, so it may not be a scanner's file as the others
are. It is here because pydicom's test data has no other image of several native frames of
16 bits. The JPEG-LS versions were made by pydicom's developers from the files they are named
after.

## Terms

pydicom ships its files in its source and binary distributions, under its MIT licence, copied
here as `LICENSE`; its `README.txt` says of the test files: "I believe there is no restriction
on using any of these files in this manner." pydicom-data is under the MIT licence copied here
as `LICENSE-pydicom-data`. These files are not under the licences of pacs-rs.
