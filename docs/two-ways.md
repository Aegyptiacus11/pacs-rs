# Two ways to use pacs-rs

pacs-rs is meant to be a research PACS and a clinical PACS in one program. The two share the
storage, the index, the viewer and the interfaces; they differ in where the images come from
and in what must be true before the first one arrives. Pick the one that is yours and read
its pages in order.

## A research archive

Beside a hospital's own PACS: studies are pulled from it for a project, de-identified on the
way in, and read by people and programs that never see the patient's identity. **This works
today.**

| Step | Read |
|---|---|
| 1. Run it and keep its data | [Running](running.md): Docker, Postgres or SQLite, disk or S3 |
| 2. Ask for a login before any real study goes in | [Login](login.md): users, roles, what each may do |
| 3. Connect the hospital's PACS | [DIMSE](dimse.md): testing the link, searching it, pulling studies |
| 4. De-identify what a project pulls | [De-identification](de-identification.md): a project, its profile, quarantine |
| 5. Let people read | [Web interface](web-interface.md): the study list, the viewer, measurements |
| 6. Let programs read and write | [REST API, task by task](rest-guide.md), [DICOMweb](dicomweb.md), and a [tool's token](login.md#tools) for each program |
| 7. Know who did what | [Audit trail](audit.md) |

What is still missing for research is in the roadmap, under [the research
workflow](roadmap.md#5-the-research-workflow).

## A clinical archive

The hospital's own archive: modalities send their images to it, and it keeps them as
received, under the patient's identity. **This is partly built.**

What works today:

| Need | Read |
|---|---|
| Receive images from modalities and workstations | [DIMSE](dimse.md): C-STORE into the archive, AE titles, who may connect |
| Read them | [Web interface](web-interface.md), [DICOMweb](dicomweb.md) for other viewers |
| A login, with roles | [Login](login.md) |
| A record of who signed in, and who stored, read, searched, exported and deleted what | [Audit trail](audit.md) |
| Bounds on what one client can take | [Limits on requests](running.md#limits-on-requests) |

What a hospital's archive needs and pacs-rs does not do yet: patient updates and orders from
the hospital's systems, Modality Performed Procedure Step, Storage Commitment, rejection
notes and retention. They are [milestone 6 of the roadmap](roadmap.md#6-the-clinical-archive),
with the IHE transactions each one follows in [the plan](interop.md#the-clinical-archive).
Other workstations cannot query or retrieve from pacs-rs over DIMSE yet either
([milestone 4](roadmap.md#4-complete-query-and-retrieve)). **Until those are done, pacs-rs
is not a replacement for a hospital's PACS.**

## Both at once

One installation that keeps a department's studies as received and serves a project's
de-identified copy, each under its own AE title and its own rights, is the last item of
milestone 6. Today a project's AE title already de-identifies what is sent to it while
everything else is stored as received ([De-identification](de-identification.md)); the
rights are not yet separate.
