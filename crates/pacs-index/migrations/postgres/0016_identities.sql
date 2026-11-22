-- The identities permission (0.16.0): a role without it sees only the studies of projects,
-- de-identified. Every role gets it, except a role held by an account limited to projects,
-- which saw only de-identified studies before and still does; admin has every permission.

UPDATE roles SET permissions = permissions || ',identities'
 WHERE id = 'admin'
    OR (id NOT IN (SELECT role FROM users WHERE ('user:' || id) IN (SELECT account FROM limited_accounts))
        AND id NOT IN (SELECT role FROM clients WHERE ('client:' || id) IN (SELECT account FROM limited_accounts)));
