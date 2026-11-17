# The sample study

The [quickstart](quickstart.md) and every tested example of this book use one small study:
three neighbouring slices of a real abdominal CT, in `docs/sample/` of the repository and at
`https://aegyptiacus11.github.io/pacs-rs/sample/0000{1,2,3}.dcm`.

## Where it comes from

The slices are from volume 0 of the training set of **LiTS**, the Liver Tumor Segmentation
Benchmark, a public research dataset of contrast-enhanced abdominal CT:

> P. Bilic et al., "The Liver Tumor Segmentation Benchmark (LiTS)", *Medical Image Analysis*
> 84, 102680, 2023. [doi:10.1016/j.media.2022.102680](https://doi.org/10.1016/j.media.2022.102680)

The paper's preprint ([arXiv:1901.04056](https://arxiv.org/abs/1901.04056), Section 3.2)
gives the data's licence as CC BY-NC-SA.

**The three files are therefore not under the licences of pacs-rs.** The code and the rest of
this book are MIT or Apache-2.0; the files of `docs/sample/`, and the pictures of this book
and of the homepage that show the scan, are adapted from LiTS and shared under
[CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/): credit the dataset,
no commercial use, and the same licence for what is made from them. Nothing of pacs-rs needs
them to build or to run.

## What was changed

LiTS gives each scan as one NIfTI volume, with the voxels and their spacing and nothing
about the patient or the examination. `scripts/sample-from-lits.py` wrote slices 64, 65 and
66 (counted from 0, from the feet) as three DICOM CT images:

```bash
python scripts/sample-from-lits.py volume-0.nii docs/sample --first 64 --count 3 \
  --patient-id LITS-0 --patient-name 'VOLUME 0^LITS' \
  --study-description 'Three slices of LiTS volume 0' --series-description 'Axial 5 mm' \
  --study-uid 2.25.3100000 --series-uid 2.25.3200000 --instance-uid-base 3300000 \
  --date 20261117
```

with nibabel 5.3.2 and pydicom 3.0.1, from the `volume-0.nii` whose MD5 is
`cbbf8359b5098d80c8d750132ba4b256`.

- **The pixels are the dataset's**, value for value, as signed 16-bit numbers; only their
  order in the file changed, to the one DICOM asks for (rows from front to back, columns
  from the patient's right to left).
- **The geometry is the dataset's**: 0.703125 mm pixels, slices 5 mm apart.
- **Everything else was given here**, because the dataset does not have it: the patient's ID
  and name (`LITS-0`), the descriptions, the UIDs, and the date, which is the day the files
  were written and not the day of the scan. The birth date, the sex, the accession number
  and the referring physician are empty.
- The window the files propose (centre 40, width 400) is a usual one for the abdomen, not a
  value from the dataset.

The test of the examples holds the SHA-256 of each file
([Building and testing](building.md#the-documentations-examples)), so the sample cannot
change without the pages that quote it being run again.
