# Modifying and merging patients

A patient registered with a wrong name, ID, birth date or sex, or registered twice (an
emergency arrival under a temporary ID, then under their own), is corrected in the archive
itself: the new values are written into every stored file of theirs, so that what is served
afterwards, over REST, DICOMweb or DIMSE, has them.

| Request | |
|---|---|
| `POST /patients/{id}/modify` | `{"PatientName", "PatientID", "PatientBirthDate", "PatientSex"}`, any of them: each given is written into every file of the patient |
| `POST /patients/{id}/merge` | `{"Into": "<the patient kept>"}`: the patient's files take the other's name, ID, birth date and sex, and their studies become the other's |

Both are for a role with `modify` ([Login](login.md#roles-and-permissions)), and for an
account not limited to projects. The web interface has them in a patient's record, under ⋯,
with an Undo. They are in the [API reference](api-reference.md#patients).

- **Values.** Each must fit its value representation: a name as DICOM writes it
  (`FAMILY^GIVEN`, five components at most, 64 characters a group), an ID of 64 characters
  at most and not empty, a birth date as `YYYYMMDD`, a sex `M`, `F` or `O`. An empty string
  removes a value, the ID's excepted. A value beyond ASCII makes the file UTF-8 (Specific
  Character Set `ISO_IR 192`). A value that does not fit is a 400 and nothing is changed.
- **The rest of each file stays.** Pixel data, transfer syntax and UIDs are as they were.
- **Identifiers.** A patient's identifier, and those of their studies, series and instances,
  are made from the Patient ID: a new one gives new identifiers, which the answer names. The
  old ones are gone, and links to them stop working. Labels and metadata move to the new
  ones.
- **Undo.** The files replaced wait in the [trash](trash.md) as an entry of level
  `Modification`, for `PACS_TRASH_DAYS`; restoring it undoes the change, whole or not at
  all: in one transaction the old files are named again, the labels and metadata go back
  (a patient merged into another keeps their own), and the entry goes. An instance deleted
  or modified again since makes the restore a 409 that changes nothing. With
  `PACS_TRASH_DAYS=0` they are removed at once and there is no undo.
- **One transaction.** The new files are written first, beside the old ones; the index then
  names them in one transaction, and the old files go to the trash. A server that stops part
  way leaves the old files named, or the new: never a file the index names that is not there.
  An instance of the patient stored while the files were rewritten, here or by another
  server sharing the index, makes the request a 409 that changes nothing: the files are
  read again on the next try.
- **Projects.** A project's de-identified copy is not modified: its patient is a pseudonym,
  made again from the original's ID each time a study arrives. The request is a 409.
- **Audit trail.** Each modification or merge is recorded as DICOM Instances Accessed,
  update, once under the patient before and once under the patient after, each with its
  birth date and sex as details of the patient ([Audit trail](audit.md)).

This is the archive's own correction. The hospital's systems send the same changes as HL7
messages (IHE RAD-12, Patient Update), which the [roadmap](roadmap.md) plans on this same
change.
