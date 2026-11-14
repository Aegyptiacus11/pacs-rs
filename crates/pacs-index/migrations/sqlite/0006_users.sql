-- The users who may sign in when the server asks for a login (PACS_AUTH), and their sessions.
-- A user's role says what they may do: reader, editor, manager or admin.

CREATE TABLE users (
    id            TEXT PRIMARY KEY,
    -- lower case; what they type to sign in
    login         TEXT NOT NULL UNIQUE,
    -- argon2id, PHC string
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL,
    -- 1, or 0 for an account that may no longer sign in
    active        INTEGER NOT NULL,
    created_ms    INTEGER NOT NULL
);

-- Refresh tokens, as the jwt-sessions crate keeps them (its SqlRefreshStore, prefix auth_):
-- a family per sign-in, and in it each token's SHA-256, never the token. Times are
-- milliseconds since the Unix epoch.

CREATE TABLE auth_refresh_families (
    id         TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    revoked_at BIGINT
);

CREATE TABLE auth_refresh_tokens (
    token_hash TEXT PRIMARY KEY,
    family_id  TEXT NOT NULL REFERENCES auth_refresh_families (id) ON DELETE CASCADE,
    issued_at  BIGINT NOT NULL,
    expires_at BIGINT NOT NULL,
    used_at    BIGINT
);

CREATE INDEX ix_auth_refresh_families_user_id ON auth_refresh_families (user_id);
CREATE INDEX ix_auth_refresh_tokens_family_id ON auth_refresh_tokens (family_id);
CREATE INDEX ix_auth_refresh_tokens_expires_at ON auth_refresh_tokens (expires_at);
