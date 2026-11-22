# De-identification

Off unless `PACS_PROJECTS` names a file of projects. A project is a set of studies, an AE title
of its own and a de-identification profile. Every study is stored once, as received, and who
reads it decides how it is shown: a role with the `identities` permission sees the study as
stored; a role without it sees the studies of its projects de-identified by each project's
profile, and nothing else ([roles](login.md#roles-and-permissions)). There is no
de-identified copy, and so no re-identification key to keep: the originals stay in the
archive, and only an account allowed to see identities sees them.

```json
[{"ae_title": "LIVER_2026",
  "profile": {"name": "liver-2026", "pseudonym_prefix": "LIV-",
              "options": ["retain-longitudinal-modified-dates", "retain-patient-characteristics",
                          "clean-descriptors"],
              "overrides": [{"tag": "(0019,xxxx)", "action": "K", "reason": "GE acquisition values"}],
              "date_shift_days": [-1095, -1],
              "extra_identifiers": ["Mustapha"]}}]
```

**A study belongs to at most one project.** It joins one when it comes in through the
project's AE title, pushed or pulled (`"Project"` in a move), and keeps it. A study already in
another project is refused there with a conflict. A study stored as received, in no
project, joins the project it is later sent to, as its instances are the same. It is also
what [rights per project](login.md#rights-per-project) go by: an account limited to some
projects sees their studies and nothing else.

## What a role without `identities` sees

- **Every attribute through the profile**: PS3.15 Table E.1-1 (2026d) whole, sequences
  included, with the options chosen: `retain-uids`, `retain-device-identity`,
  `retain-institution-identity`, `retain-patient-characteristics` (Patient's Age added from
  the dates), `retain-longitudinal-full-dates`, `retain-longitudinal-modified-dates` (one shift
  per patient, every date, times kept) and `clean-descriptors` (text kept unless it holds the
  patient's names or identifiers, the people and institution, or `extra_identifiers`).
  `overrides` change any attribute's action, private ones included: `{"tag": "(0010,0020)",
  "action": "K"}` shows the hospital's Patient ID, and the De-identification Method names it,
  so the data reads as pseudonymised, not de-identified. `remove_when_allowed` takes X
  wherever the table allows it. A profile pacs-rs cannot apply is refused at start.
- **The same replacements on every route**: searches, metadata, instances, archives, the web
  interface and the project's AE title over DIMSE. UIDs, pseudonyms (Patient ID and Name)
  and date shifts are derived with HMAC from the project's secret, in
  `PACS_PROJECT_SECRET_<AE title>` (or `..._FILE`), at least 32 bytes (`openssl rand -hex 32`):
  the same patient and UIDs map the same way every time, and studies, series and references
  stay linked. The index keeps each study's, series' and instance's replaced UID, so a
  request naming one finds the stored instance.
- **Searches on what it cannot see are refused** (400): matching on the patient's name, ID,
  birth date, the accession number or another attribute the profile replaces or removes
  would tell who is in the archive. Other keys (modality, body part, the shifted dates as
  shown) match as usual.
- **Pixel data as stored.** The profile does not change pixels, so frames, rendered images
  and bulk data are served as they are.
- **Hidden instances**: what may carry identification the profile cannot remove is not shown
  to it: Secondary Capture, encapsulated documents, SR, presentation states, ultrasound,
  Burned In Annotation YES, Image Type SCREEN SAVE (each profile may change the list,
  `"quarantine"`). It stays in the archive, seen by roles with `identities`.
- **Recorded in each instance it gets**: Patient Identity Removed YES, the De-identification
  Method Code Sequence (CID 7050) and a De-identification Method naming the profile and each
  override, new File Meta Information.

## Results coming back

A segmentation or a report a research tool makes from what it saw refers to the replaced
UIDs. Stored by an account without `identities` (STOW-RS, or C-STORE to the project's AE
title), its references are mapped back to the stored UIDs and it joins the original study,
where a role with `identities` sees it as it would any other instance, and
[forwarding](dimse.md#forwarding-by-rule) can send it on to the hospital.

## Upgrading from 0.15

Studies a project received before 0.16.0 were stored de-identified, and are shown as stored.
Every role gets `identities`, except the roles of accounts limited to projects, so that what
each account sees does not change; `admin` has every permission, `identities` included.
`PACS_QUARANTINE` is no longer read: the server refuses to start while it is set, so that
its files are not forgotten. They are identified originals; send them to a project's AE
title to have them in the archive.
