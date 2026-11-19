# Test files from pydicom

Two DICOM files made by scanners, taken unchanged from the test data of
[pydicom](https://github.com/pydicom/pydicom) 3.0.2 (`pydicom/data/test_files`), for the
tests that store, search and send real DICOM (`pacs_core::real`).

| File | What it is | SHA-256 |
|---|---|---|
| `CT_small.dcm` | CT Image Storage, GE Medical Systems, Explicit VR Little Endian, 128 x 128, 16 bits | `3dd31e5cc835b3f2cdd46c9da1982f59251e78518fefa8163d914631c66437d6` |
| `MR_small.dcm` | MR Image Storage, Toshiba, Explicit VR Little Endian, 64 x 64, 16 bits | `3f27d1c22f1a66e80d7bb7c911e8610fd0bb70325a76746a7adb1c0ddefcf2bb` |

Origin, as pydicom's own `README.txt` beside them says: `CT_small.dcm` was downsized to
128 x 128 from `CT1_UNC`, and `MR_small.dcm` to 64 x 64 from `MR1_UNC`, both of the NEMA
WG04 compression sample set (`ftp://medical.nema.org/MEDICAL/Dicom/DataSets/WG04/`). The
patient names in them are the sample set's (`CompressedSamples^CT1`, `CompressedSamples^MR1`).

Terms: pydicom ships these files in its source and binary distributions, under its MIT
licence, copied here as `LICENSE`. Its `README.txt` says of the test files: "I believe there
is no restriction on using any of these files in this manner." These two files are not
under the licences of pacs-rs.
