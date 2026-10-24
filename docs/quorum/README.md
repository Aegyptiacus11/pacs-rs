# pacs-rs behind Quorum

Quorum (the CT annotation platform) talks to its PACS over DICOMweb (the viewer, CT download)
and over the REST API (upload, study lookup, statistics, archives). `pacs-rs.override.yml`
runs Quorum's dev stack with pacs-rs as that PACS, under its own Compose project and volume
names so the regular dev data is never touched:

```bash
docker build -t pacs-server:small .           # in this repo
cd <quorum checkout>
cp ops/.env.example /tmp/quorum.env           # dev-only values
docker compose -p quorum-pacsrs --env-file /tmp/quorum.env \
  -f ops/compose.dev.yml -f <pacs-rs>/docs/quorum/pacs-rs.override.yml \
  up -d --build frontend backend
# Quorum on http://localhost:3000, pacs-rs on 127.0.0.1:18142
docker compose -p quorum-pacsrs --env-file /tmp/quorum.env \
  -f ops/compose.dev.yml -f <pacs-rs>/docs/quorum/pacs-rs.override.yml down -v
```

pacs-rs keeps its files in Quorum's MinIO (S3) and its index in SQLite; Quorum runs with
`DICOM_INGEST=orthanc`, the setup where the PACS holds the only copy of each CT.

## 2026-10-24: real CT through Quorum

Quorum `feat/wadors-primary` at `5d9aa04`, pacs-rs at "pacs-server --healthcheck", with three
3D-IRCADb-01 patients (public research data, CC BY-NC-ND, used locally only):

| What | Result |
|---|---|
| Patient 1 (129 slices, Explicit VR LE, empty PatientID, InstanceNumber 0) uploaded through Quorum's API | 129/129 stored; dataset, case and case page (slices, matrix, spacing) correct |
| The same case in Quorum's viewer | rendered in axial, sagittal and coronal; 135 DICOMweb requests (QIDO, series metadata, every frame) all 200; no pacs-rs errors in 489 requests |
| Patient 2 (172 slices) uploaded through Quorum's Add case dialog | 172/172 stored, case page correct, case opened |
| Patient 3 re-encoded JPEG-LS lossless (200 slices, 100 -> 30 MB, each slice checked to decode back exactly) | stored as is, frames served as `image/jls`, decoded by the viewer with correct HU |
| Quorum's CT download (DICOM zip, over WADO-RS) for patient 2 | 172/172 files byte-identical to the originals, 1.1 s |
| Quorum's `orthanc.study_size` and `find_study_id` (REST) | exact byte totals and instance counts for both studies |
| Case geometry from Quorum's archive read (`/tools/find` + `/studies/{id}/archive`) | correct for the uncompressed study |

Two things did not work, both Quorum's own, and they would happen with any PACS behind it:

- Its upload compressed none of these files: its DICOM writer refuses a DS value longer than
  16 characters (`1.6000000238418579` in IRCADb), so each file went up uncompressed.
- In this ingest mode it captures a case's geometry by decoding the CT in the API image, which
  has no JPEG-LS decoder, so a JPEG-LS study - what its own upload produces - gets no geometry,
  and the case page no study facts.
