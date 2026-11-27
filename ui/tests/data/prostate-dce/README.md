# prostate-dce

Ten MR images, one series, as scanned (nothing altered): series "DCE 262 pre" of patient
MIP-PROSTATE-01-0001 of the PROSTATE-MRI collection of The Cancer Imaging Archive,
doi:10.7937/K9/TCIA.2016.6046GUDV, licensed CC BY 3.0.

- Series Instance UID: 1.3.6.1.4.1.14519.5.2.1.9823.1001.165695987073963198817650624908
- 10 axial slices of 256 x 256, 16 bits allocated, 12 stored, Explicit VR Little Endian
- Rescale Slope 6.197..., a window per image, VOI LUT Function LINEAR (Philips)
- Downloaded on 24 November 2026 from the Imaging Data Commons public bucket (index v25)
  with the `idc-index` Python package

The viewer's browser tests use it (through `../../fixtures/viewer-dce.har`, recorded by
`../../record-viewer.mjs prostate-dce`): a series of at least 8 slices for a reformatted plane, and the
images that showed the VOI LUT Function bug.
