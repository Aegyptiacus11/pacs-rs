"""Writes slices of a NIfTI CT volume as a DICOM CT series (one file per slice).

This is how docs/sample was made from volume 0 of LiTS (docs/sample/README.md has the
command). Needs nibabel, numpy and pydicom.

    python scripts/sample-from-lits.py volume.nii out_dir --first K --count N [--step S]
        --patient-id ID --patient-name NAME --study-description TEXT --series-description TEXT
        --study-uid UID --series-uid UID --instance-uid-base N (instance n is 2.25.<N + n>)

Fails (non-zero exit) when the volume is not 3D int16, not axis-aligned, or the slices asked
for are outside it. The pixels are written as stored, with no rescaling.
"""

import argparse
import sys
from pathlib import Path

import nibabel as nib
import numpy as np
from pydicom.dataset import Dataset, FileMetaDataset
from pydicom.uid import CTImageStorage, ExplicitVRLittleEndian


def lps_slices(image):
    """The volume as (slice, row, column) with rows to posterior and columns to the left,
    slices to superior, and the spacing (row, column, slice)."""
    canonical = nib.as_closest_canonical(image)  # RAS+
    affine = canonical.affine
    rotation = affine[:3, :3]
    zooms = np.array(canonical.header.get_zooms()[:3], dtype=float)
    if not np.allclose(rotation, np.diag(zooms), atol=1e-3 * zooms.max()):
        sys.exit(f"the volume is not axis-aligned: {rotation.tolist()}")
    slope, intercept = image.header.get_slope_inter()
    if slope not in (None, 1.0) or intercept not in (None, 0.0):
        sys.exit(f"the volume is scaled (slope {slope}, intercept {intercept}): not handled")
    scaled = np.asanyarray(canonical.dataobj)
    data = scaled.astype(np.int16)
    if data.ndim != 3 or image.get_data_dtype() != np.int16 or not np.array_equal(data, scaled):
        sys.exit(f"expected a 3D int16 volume, found {data.shape} {image.get_data_dtype()}")
    # RAS+ (x to right, y to anterior) -> rows A to P, columns R to L
    stack = data[::-1, ::-1, :].transpose(2, 1, 0)
    nx, ny, _ = data.shape
    # LPS position of the first pixel (most right, most anterior) of slice 0
    corner = affine @ np.array([nx - 1, ny - 1, 0, 1.0])
    origin = np.array([-corner[0], -corner[1], corner[2]])
    return stack, zooms, origin


def number_of(value):
    """A number as a Decimal String, without trailing zeros."""
    return f"{float(value):.6f}".rstrip("0").rstrip(".")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("volume")
    p.add_argument("out")
    p.add_argument("--first", type=int, required=True)
    p.add_argument("--count", type=int, required=True)
    p.add_argument("--step", type=int, default=1)
    p.add_argument("--patient-id", required=True)
    p.add_argument("--patient-name", required=True)
    p.add_argument("--study-description", required=True)
    p.add_argument("--series-description", required=True)
    p.add_argument("--study-uid", required=True)
    p.add_argument("--series-uid", required=True)
    p.add_argument("--instance-uid-base", type=int, required=True)
    p.add_argument("--date", required=True)
    a = p.parse_args()

    stack, zooms, origin = lps_slices(nib.load(a.volume))
    chosen = [a.first + i * a.step for i in range(a.count)]
    if a.count < 1 or a.step < 1 or chosen[0] < 0 or chosen[-1] >= stack.shape[0]:
        sys.exit(f"slices {chosen[0]}..{chosen[-1]} are not in a volume of {stack.shape[0]}")
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=False)

    for number, k in enumerate(chosen, start=1):
        pixels = np.ascontiguousarray(stack[k])
        ds = Dataset()
        ds.file_meta = FileMetaDataset()
        ds.file_meta.MediaStorageSOPClassUID = CTImageStorage
        instance = f"2.25.{a.instance_uid_base + number}"
        ds.file_meta.MediaStorageSOPInstanceUID = instance
        ds.file_meta.TransferSyntaxUID = ExplicitVRLittleEndian
        ds.SOPClassUID = CTImageStorage
        ds.SOPInstanceUID = instance
        ds.StudyInstanceUID = a.study_uid
        ds.SeriesInstanceUID = a.series_uid
        ds.FrameOfReferenceUID = a.series_uid + ".1"
        ds.PatientName = a.patient_name
        ds.PatientID = a.patient_id
        ds.PatientBirthDate = ""
        ds.PatientSex = ""
        ds.StudyDate = a.date
        ds.StudyTime = "120000"
        ds.AccessionNumber = ""
        ds.ReferringPhysicianName = ""
        ds.StudyID = "1"
        ds.StudyDescription = a.study_description
        ds.SeriesDescription = a.series_description
        ds.Modality = "CT"
        ds.SeriesNumber = 1
        ds.InstanceNumber = number
        ds.ImageType = ["DERIVED", "SECONDARY", "AXIAL"]
        ds.PatientPosition = "HFS"
        ds.ImageOrientationPatient = ["1", "0", "0", "0", "1", "0"]
        position = origin + np.array([0, 0, k * zooms[2]])
        ds.ImagePositionPatient = [number_of(v) for v in position]
        ds.SliceLocation = number_of(position[2])
        ds.PixelSpacing = [number_of(zooms[1]), number_of(zooms[0])]
        ds.SliceThickness = number_of(zooms[2] * a.step)
        ds.SamplesPerPixel = 1
        ds.PhotometricInterpretation = "MONOCHROME2"
        ds.Rows, ds.Columns = pixels.shape
        ds.BitsAllocated = 16
        ds.BitsStored = 16
        ds.HighBit = 15
        ds.PixelRepresentation = 1
        ds.RescaleIntercept = "0"
        ds.RescaleSlope = "1"
        ds.RescaleType = "HU"
        ds.WindowCenter = "40"
        ds.WindowWidth = "400"
        ds.PixelData = pixels.astype("<i2").tobytes()
        ds.save_as(out / f"{number:05d}.dcm", enforce_file_format=True)
    print(f"{len(chosen)} slices of {stack.shape} written to {out}")


if __name__ == "__main__":
    main()
