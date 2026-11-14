# De-identification

Off unless `PACS_PROJECTS` names a file of projects. A project is an AE title of its own and a
profile: whatever is sent to that AE title, pushed or pulled (`"Project"` in a move), is
de-identified on the way in, and only the result is stored, indexed and served. The identified
instance is held in memory meanwhile, never written. Everything else is stored as received.

```json
[{"ae_title": "LIVER_2026",
  "profile": {"name": "liver-2026", "pseudonym_prefix": "LIV-",
              "options": ["retain-longitudinal-modified-dates", "retain-patient-characteristics",
                          "clean-descriptors"],
              "overrides": [{"tag": "(0019,xxxx)", "action": "K", "reason": "GE acquisition values"}],
              "date_shift_days": [-1095, -1],
              "extra_identifiers": ["Mustapha"]}}]
```

- **Profile**: PS3.15 Table E.1-1 (2026d) whole, sequences included, with the options chosen:
  `retain-uids`, `retain-device-identity`, `retain-institution-identity`,
  `retain-patient-characteristics` (Patient's Age added from the dates), `retain-longitudinal-full-dates`,
  `retain-longitudinal-modified-dates` (one shift per patient, every date, times kept) and
  `clean-descriptors` (text kept unless it holds the patient's names or identifiers, the people and
  institution, or `extra_identifiers`). `overrides` change any attribute's action, private ones included;
  `remove_when_allowed` takes X wherever the table allows it. A profile pacs-rs cannot apply is refused at
  start.
- **Replacement values**: UIDs, pseudonyms (Patient ID and Name) and date shifts are derived with HMAC from
  the project's secret, in `PACS_PROJECT_SECRET_<AE title>` (or `..._FILE`), at least 32 bytes
  (`openssl rand -hex 32`): the same patient and UIDs map the same way on every pull, and studies, series and
  references stay linked. Keep the secret as safe as the data.
- **Recorded**: Patient Identity Removed YES, the De-identification Method Code Sequence (CID 7050) and a
  De-identification Method naming the profile and each override, new File Meta Information.
- **Quarantine**: what may carry identification pacs-rs cannot remove is not de-identified: Secondary
  Capture, encapsulated documents, SR, presentation states, ultrasound, Burned In Annotation YES, Image Type
  SCREEN SAVE (each profile may change the list). It is kept as received in `PACS_QUARANTINE`, never indexed
  or served, or without it refused (C-STORE status C003, with the reason).

Not yet: a re-identification key (the Encrypted Attributes Sequence of PS3.15 E.1.1, or an encrypted
table), Retain Safe Private, Clean Structured Content, Clean Graphics, Clean Pixel Data, a review of the
quarantine in the web interface, and a run on the MIDI-B validation data. The plan is in
[the hospital interfaces plan](interop.md).
