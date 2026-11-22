-- The Patient ID a move names its studies under, for a remote AE that retrieves in Patient
-- Root only (PS3.4 C.4.2.2.1); NULL when none was given.

ALTER TABLE jobs ADD COLUMN patient_id TEXT;
