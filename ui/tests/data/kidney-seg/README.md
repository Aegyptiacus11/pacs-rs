# kidney-seg

Eight CT slices of one series, a DICOM Segmentation and an RT Structure Set drawn on that
series, as published (one file derived, below): patient C3L-01958 of the CPTAC-CCRCC collection
of The Cancer Imaging Archive, licensed CC BY 3.0, downloaded on 27 November 2026 from the
Imaging Data Commons public bucket (index v23) with the `idc-index` Python package.

- `ct/`: slices 65 to 72 (z -50.5 to -64.5 mm, 2.5 mm thick, 2 mm apart) of series
  "Recon 2: +C", 1.3.6.1.4.1.14519.5.2.1.6450.2626.275857414129959716213130533744; 512 x 512,
  Explicit VR Little Endian, GE.
- `seg.dcm`: "AIMI kidney tumor cyst AI segmentation" (BAMF Health AI model, IDC's AIMI
  annotations), series 1.2.276.0.7230010.3.1.3.17436516.3852943.1720464955.16841; BINARY, three
  segments (Kidney, Mass, Cyst), 101 frames over 50 slices of the series, 24 of them on the
  eight slices kept.
- `rtstruct.dcm`: "Pre-Dose, RIGHT KIDNEY - 1", series
  1.2.826.0.1.534147.756.812677238.20232169519195.4; one region, RT KIDNEY - 1, yellow, 14
  closed contours, one on each slice from -44.5 to -70.5 mm, so one on each slice kept.

- `seg-fractional.dcm`: made from `seg.dcm` with pydicom 3.0.1, the one file here that is
  altered: frames 51 (Mass) and 1 (Kidney) only, as a FRACTIONAL SEG of type PROBABILITY,
  8 bits, Maximum Fractional Value 255, frame 51's mask at 200 (in) and frame 1's at 100
  (under half: out); new SOP Instance and Series UIDs, " (fractional, derived)" added to the
  Series Description. No real fractional SEG of this series is published.

The slices were chosen where all three segments and the contour are present, eight so that
the viewer can reformat them. The viewer's browser tests and `../../segmentation.spec.ts` use
them; that test's figures were read from the same files with pydicom 3.0.1.
