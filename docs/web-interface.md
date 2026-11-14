# Web interface

At `/ui/` (`/` redirects there), on the same REST API and DICOMweb as any other client. Built with
React, [shadcn/ui](https://ui.shadcn.com/) (Radix primitives, Tailwind) and lucide icons; the
viewer is [Cornerstone3D](https://www.cornerstonejs.org/) 5.14.0.

- **Layout**: a rail down the left side to go places (Studies, Patients), a bar along the top
  for the tools (the search, light or dark, a warning when the server doesn't answer), the page
  in between.
- **Studies**: a worklist and, beside it, the chosen study. One row of filters over the table:
  a filter box (`/`), the date, the modality (every DICOM imaging modality), the sex and the age
  at the study (0–17, 18–39, 40–64, 65+), each with how many studies it holds. The date opens
  a small panel: today, yesterday, this week, this month and this year, each with its studies
  (clicked again, it lets go), and From
  and To fields, typed (in the browser's day, month, year order, or year first) or picked on a
  small calendar under each (the days with studies dotted). Sortable columns (date first, then patient, study,
  modality, series, images). A click on a row shows the
  study in the pane: the patient, its series as thumbnails (a click opens one), the patient's
  other studies (prior or later), its details, and the actions: open, download (ZIP), and under
  ⋯ copy the Study UID, the patient's studies, download everything of the patient, delete the
  study or the patient. A double click or Enter opens it; ↑ ↓
  move through the list. With no study chosen, the pane sums up what the list holds, for the
  dates and filters chosen (patients, studies, series, images, studies by modality, the
  last received). The footer says how much of the list is shown (1–50
  of 1,234 studies), takes 25, 50 (the default), 100 or 200 rows a page, and goes to the
  first, previous, next or last page, a numbered one, or one typed in; ↑ ↓ go on to the next or previous page. The server filters, sorts, counts and pages the list
  (`/ui/api/studies`), so the browser only ever holds one page, however large the archive. Checkboxes download or delete several
  studies at once (one ZIP from `/tools/create-archive`).
- **Search**: two, with two jobs. The top bar's (`Ctrl K`, `⌘K` on a Mac) finds a patient or a
  study anywhere in the archive and shows it on the page already open: on the studies, the
  patient's studies or the study itself; on the patients, the patient's record. Each list has
  its own box (`/`) that filters that list as words are typed and never leaves the page.
- **Patients**: a list (latest study first, by name or by number of studies, a page at a time
  from `/ui/api/patients`) and the chosen patient's record: who they are, their totals
  (studies, series, images, first study, modalities), and a timeline of their studies, newest
  first, each with its series as thumbnails. Tick two studies to compare them: the viewer shows
  each one's largest series side by side, scrolling together. Download everything of the
  patient, or delete them.
- **Light or dark**: one button on the rail, remembered in the browser; the system setting until it is used. The viewer is always dark.
- **Upload**: a dialog with a drop zone, or files and folders dropped anywhere; each file goes
  to `POST /instances` and is counted as new, already stored or skipped.
- **Viewer**: full screen and dark, laid out like a reading workstation. A toolbar of labelled
  tools: window/level with presets and invert, pan, zoom, a magnifying glass (held over a spot, 1.5 to 8 times with the wheel), measurements
  (length, pixel value, angle, Cobb angle, bidirectional, elliptical and rectangle ROI), rotate
  and flip, Add view (the active series in the next plane, sagittal then coronal, or the next
  series), Views (up to 2 × 3, 3 planes, scrolling the viewports together: Sync), cine, DICOM
  tags, export (PNG of the view, or the DICOM file), full screen. Each viewport has a plane menu
  (as acquired, axial, sagittal, coronal, 3D) and can be removed. A 3D view renders the
  series' volume (CT: bone, muscle and bone, vessels, lung, skin, MIP; MR: default, angio, T2
  brain, MIP); drag to turn it, the wheel or a right drag zooms. On the left, the patient's studies (current, priors and later
  ones) and their series: a click shows one in the active viewport, a drag shows it in any.
  Each viewport has its own text, orientation letters, a centimetre scale, its position in
  the series along the top, and a preset menu. MPR shows the series as a volume, axial,
  sagittal and coronal with crosshairs, and a full-thickness coronal MIP; each view takes a
  thin slice or a MIP slab of 5, 10 or 20 mm. Frames come over WADO-RS as stored and are
  decoded in the browser (JPEG-LS, JPEG, JPEG 2000, HTJ2K, JPEG XL). MPR, other planes and 3D need WebGL; the rest
  also renders without it ("Software rendering" under Views, turned on by itself when a
  browser's WebGL draws blank). Keys follow the names: W window, P pan, Z zoom, M magnifier,
  L length, A angle, C Cobb, B bidirectional, E ellipse, ⇧E rectangle, D pixel value (density),
  R rotate (⇧R left), H and V flip, I invert, 0 reset, N new view, X 3 planes, S sync; `?`
  lists them all.
- **Hanging protocols**: a study opens as the best protocol that fits it lays it out: the
  current study beside its latest prior (the prior's most similar series, scrolling linked),
  two views for CR, DX, MG and RF, the largest series, every series, or MPR. The toolbar lists
  the ones that fit, with previous and next (`,` `.`). "Save this layout as a protocol" keeps
  the arrangement by kind of series (current or prior, modality, description), not by UID, so
  it serves every matching study. Saved protocols are kept by the server, in the index, so
  every browser and everyone using the server shares them; they come first when they fit.
  Opening one series (a thumbnail) shows that series alone.
- **Measurements are kept**: each one is saved with its study as soon as it is drawn, changed
  or removed, and is there again when the study is opened, in any browser. Clear removes them
  all, with an undo.

The shared protocols are at `/ui/api/protocols`: `GET` lists them (most recently saved first),
`PUT /ui/api/protocols/{id}` keeps one (a JSON object whose `id` is the path's, at most 64 KiB;
at most 1000 protocols), `DELETE` removes one. They are the interface's own documents, stored as
received in the index's `ui_protocols` table.

The study list is at `/ui/api/studies`: `q` (words, each found in the patient's name or ID,
the accession number, the description, the referring physician or the date), `from` and `to`
(`YYYYMMDD`, included), `modality` and `sex` (comma-separated; `O` for other), `age` (the age at the study, ranges of
years: `0-17,65-`), `patient` (a
public ID), `sort` (`patient`, `description`, `modality`, `date`, `series`, `images`,
`received`), `order` (`asc`, `desc`), `offset`, `limit` (at most 1000) and `summary=true` (the
whole list's totals and its last received studies). It answers `{"Total", "Studies",
"Summary"?}`; `/ui/api/studies/facets[?patient=]` gives the filters' choices with their counts.
`/ui/api/patients` lists patients the same way (`q`, `sort`: `name`, `last` or `studies`,
`order`, `offset`, `limit`), each with its study and image counts, modalities and first and last
study dates.

The measurements are at `/ui/api/annotations/{StudyInstanceUID}`: `GET` returns the study's list
(a JSON array, empty when it has none), `PUT` replaces it (an array of objects, at most 1 MiB; an
empty one removes it), for a study the server has. They are Cornerstone3D annotations, stored as
received in the `ui_annotations` table; deleting the study deletes them.

Without the [login](login.md), whoever reaches it can delete, and change the shared protocols and
the measurements. With it, the interface cannot be used yet: it has no sign-in page. `PACS_UI=false` turns it off, protocols and measurements included.
