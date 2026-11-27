-- The permission to modify patients (correct a name, an ID, a birth date or a sex, or merge
-- two patients), which rewrites their stored files. The admin role permits everything; the
-- others get it when an administrator ticks it for them.

UPDATE roles SET permissions = 'read,write,remote,delete,modify,users' WHERE id = 'admin';
