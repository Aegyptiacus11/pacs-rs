# The DICOM audit message schema

- `dicom-audit-message.rnc`: the schema of DICOM PS3.15 A.5.1.1 (2026d), as the standard
  prints it, in RELAX NG Compact.
- `dicom-audit-message.rng`: the same in RELAX NG XML, which `xmllint` reads. Made with
  `rnc2rng` after turning the schema's `##` comments into `#` (they follow the values they
  describe, where RELAX NG Compact wants a documentation comment before them); nothing else
  was changed.

`schema.rs` validates every kind of message this crate builds against it, when
`PACS_TEST_XMLLINT` names the `xmllint` program (CI does; without it the test is skipped):

```bash
PACS_TEST_XMLLINT=xmllint cargo test -p pacs-audit
```
