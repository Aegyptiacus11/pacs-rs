-- Roles, each a name and the permissions it gives: read, write, remote, delete, users. A
-- user's role (users.role) is a role's id. The four below are what every server starts with;
-- all but admin can be changed or removed, and others added.

CREATE TABLE roles (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    -- comma-separated
    permissions TEXT NOT NULL,
    created_ms  INTEGER NOT NULL
);

INSERT INTO roles (id, name, permissions, created_ms) VALUES
    ('reader',  'Reader',  'read', 0),
    ('editor',  'Editor',  'read,write,remote', 1),
    ('manager', 'Manager', 'read,write,remote,delete', 2),
    ('admin',   'Admin',   'read,write,remote,delete,users', 3);
