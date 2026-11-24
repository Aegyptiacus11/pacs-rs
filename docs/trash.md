# Deleting and restoring

A deletion can be undone for `PACS_TRASH_DAYS` days (30 by default). The deleted patient,
study, series or instance is gone from the index at once: nothing lists, searches or serves
it, over REST, DICOMweb or DIMSE. Its files wait in the store, under `trash/`, and the
[audit trail](audit.md) records the deletion as usual.

| Request | |
|---|---|
| `GET /trash` | the deletions waiting, the latest first: what was deleted, its studies (UID, patient, project), how many files, when it was deleted and when it will be emptied |
| `POST /trash/{id}/restore` | put it back: its files are indexed again as if received, each study in the project it was in, with the viewer's annotations of a whole study; the audit trail records the instances as received |
| `DELETE /trash/{id}` | empty it now: its files are removed for good |

These are for a role with `delete`, and for an account not limited to projects: the trash
holds what every project deleted. They are in the [API reference](api-reference.md#trash).

- Once an hour, the server empties the deletions older than `PACS_TRASH_DAYS`.
- An instance stored again between the deletion and the restore keeps the newer copy: the
  restore leaves that file in the trash and says so, to be emptied.
- `PACS_TRASH_DAYS=0` turns the trash off: a deletion removes the files at once, as before
  0.13.
- The trash takes room in the store until it is emptied. The
  [consistency check](integrity.md#the-store-and-the-index-compared) leaves it alone.
- The index forgets a resource and lists its files for the trash in one transaction, then
  moves them. A server that stops in between leaves files where they were, still the
  trash's: restoring or emptying the entry finds them there, and the consistency check does
  not take them for orphans.
