"""Tests of sample-from-lits.py: the DICOM files it writes, and the volumes it refuses.

    uv run --with nibabel==5.3.2 --with numpy --with pydicom==3.0.1 python -m unittest scripts/test_sample_from_lits.py
"""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import nibabel as nib
import numpy as np
import pydicom

SCRIPT = Path(__file__).parent / "sample-from-lits.py"
NAMES = [
    "--patient-id", "P1", "--patient-name", "A^B", "--study-description", "study",
    "--series-description", "series", "--study-uid", "2.25.1", "--series-uid", "2.25.2",
    "--instance-uid-base", "300", "--date", "20260102",
]


def volume(directory, data, affine):
    path = Path(directory) / "volume.nii"
    nib.save(nib.Nifti1Image(data, affine), path)
    return path


def las(spacing=(0.5, 0.5, 2.0)):
    """The affine of LiTS: first axis to the left, second to the front, third to the head."""
    return np.diag([-spacing[0], spacing[1], spacing[2], 1.0])


def convert(path, out, *range_):
    return subprocess.run(
        [sys.executable, str(SCRIPT), str(path), str(out), *range_, *NAMES],
        capture_output=True, text=True,
    )


class SampleFromLits(unittest.TestCase):
    def test_the_slices_are_written_as_dicom_asks(self):
        with tempfile.TemporaryDirectory() as directory:
            data = np.arange(4 * 3 * 5, dtype=np.int16).reshape(4, 3, 5) - 30
            path = volume(directory, data, las())
            out = Path(directory) / "out"
            done = convert(path, out, "--first", "1", "--count", "2")
            self.assertEqual(done.returncode, 0, done.stderr)
            files = sorted(out.glob("*.dcm"))
            self.assertEqual([f.name for f in files], ["00001.dcm", "00002.dcm"])

            first = pydicom.dcmread(files[0])
            self.assertEqual((first.Rows, first.Columns), (3, 4))
            self.assertEqual(first.PixelRepresentation, 1)
            # rows from front to back, columns from the patient's right to left: the first
            # axis of LiTS already runs to the left, the second runs to the front
            np.testing.assert_array_equal(first.pixel_array, data[:, ::-1, 1].T)
            self.assertEqual([float(v) for v in first.PixelSpacing], [0.5, 0.5])
            self.assertEqual(float(first.SliceThickness), 2.0)
            self.assertEqual(float(first.ImagePositionPatient[2]), 2.0)
            self.assertEqual(first.SOPInstanceUID, "2.25.301")
            self.assertEqual((first.PatientID, first.StudyDate), ("P1", "20260102"))

            second = pydicom.dcmread(files[1])
            self.assertEqual(float(second.ImagePositionPatient[2]), 4.0)
            self.assertEqual(second.SOPInstanceUID, "2.25.302")
            self.assertEqual(second.SeriesInstanceUID, first.SeriesInstanceUID)

    def refused(self, data, affine, why, *range_):
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory) / "out"
            done = convert(volume(directory, data, affine), out, *range_)
            self.assertEqual(done.returncode, 1, done.stderr)
            self.assertIn(why, done.stderr)
            self.assertFalse(out.exists() and any(out.iterdir()), "files were written")

    def test_slices_outside_the_volume_are_refused(self):
        data = np.zeros((4, 3, 5), dtype=np.int16)
        self.refused(data, las(), "are not in a volume of 5", "--first", "4", "--count", "2")

    def test_a_volume_that_is_not_16_bit_integers_is_refused(self):
        data = np.zeros((4, 3, 5), dtype=np.float32)
        self.refused(data, las(), "expected a 3D int16 volume", "--first", "0", "--count", "1")

    def test_a_tilted_volume_is_refused(self):
        tilted = las()
        tilted[0, 1] = 0.2
        data = np.zeros((4, 3, 5), dtype=np.int16)
        self.refused(data, tilted, "not axis-aligned", "--first", "0", "--count", "1")

    def test_a_folder_that_exists_is_not_written_over(self):
        with tempfile.TemporaryDirectory() as directory:
            path = volume(directory, np.zeros((4, 3, 5), dtype=np.int16), las())
            out = Path(directory) / "out"
            out.mkdir()
            done = convert(path, out, "--first", "0", "--count", "1")
            self.assertNotEqual(done.returncode, 0)


if __name__ == "__main__":
    unittest.main()
