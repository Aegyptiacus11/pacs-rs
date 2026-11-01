# Benchmarks

`scripts/bench.sh` against pacs-rs and the reference server (see the README, "Comparing with a
reference server"), with the same client and the same files.

## 2026-10-24: memory while serving a study ZIP

`scripts/zip-memory.sh`: peak resident memory (VmHWM) of the pacs-server binary, native, before
and after `GET /studies/{id}/archive`, for the synthetic CT of 200 and of 600 slices.

| | before: ZIP built in memory | after: ZIP streamed |
|---|---|---|
| 200 slices, 101 MB: added peak memory | +255 to +259 MB | +14 to +26 MB |
| 200 slices: first byte / whole ZIP | 1.70-1.84 s / 1.82-1.96 s | 7 ms / 1.78-1.92 s |
| 600 slices, 303 MB: added peak memory | +556 MB | +32 MB |
| 600 slices: first byte / whole ZIP | 6.10 s / 6.44 s | 19 ms / 5.68 s |

Before, memory grew with the study (about 1.8 times its size); streamed, it holds one file at a
time whatever the study's size.

## 2026-10-24: one CT series, reference server 26.9.1 (latest)

The same workload and machine as below, against the latest reference release (26.9.1, core
1.13.0), three rounds:

| | reference 26.9.1 | pacs-rs |
|---|---|---|
| upload 200 files, 4 at a time | 0.87 s | 0.68 s |
| series metadata, first request | 75 ms | 3 ms |
| series metadata, later requests (median of 5) | 6-11 ms | 2-3 ms |
| 200 frames, one request after another | 1.93-1.97 s | 1.74-1.81 s |
| 200 frames, 8 requests at a time | 0.29-0.30 s | 0.27-0.28 s |
| study ZIP, 95 MB | 2.49-2.50 s | 1.92-1.98 s |

The pacs-rs figures above are from the Debian-based image. The static image that replaced it
(15 MB, musl with mimalloc) measured the same or better on this workload: frames 1.72-1.84 s
one at a time and 0.26-0.27 s eight at a time, the ZIP 1.72-1.76 s. Without mimalloc, musl's
allocator made frames about 25% slower.

## 2026-10-24: one CT series, reference server 26.6.1

- Data: a synthetic series, 200 slices of 512 x 512 x 16 bits, 101 MB
  (`write_synthetic -- ct 77 200 512`).
- Both servers in Docker on one laptop (AMD Ryzen 7 PRO 7840U, 16 threads, 30 GB, Docker 29.9.2,
  overlay2), each fresh, files and index on the container's disk, curl on the same machine.
- pacs-rs at commit "WADO-RS metadata: send the stored JSON as it is"; the reference server in
  version 26.6.1 with its default configuration and its DICOMweb plugin.
- Three rounds; the upload only in the first (later rounds re-send stored instances).

| | reference | pacs-rs |
|---|---|---|
| upload 200 files, 4 at a time | 0.77 s | 0.67 s |
| series metadata, first request | 76 ms | 3 ms |
| series metadata, later requests (median of 5) | 5-6 ms | 2-3 ms |
| 200 frames, one request after another | 1.99-2.07 s | 1.70-1.73 s |
| 200 frames, 8 requests at a time | 0.28-0.29 s | 0.27-0.28 s |
| study ZIP, 95 MB | 2.49-2.52 s | 1.89-1.90 s |

What this does not say: anything about other data (real CT, compressed transfer syntaxes,
thousands of studies), other hardware, S3 storage, or a tuned reference server. It is one
workload, the one a viewer opening a CT series makes, measured once on one machine.

The first run of this benchmark found a bug: concurrent uploads failed with "database is
locked" (fixed in "pacs-index: take the write lock up front").
