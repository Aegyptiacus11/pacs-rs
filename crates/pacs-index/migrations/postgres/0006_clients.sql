-- Clients: programs that connect without a person (a viewer, a pipeline). A client presents
-- its id and secret to get a short access token (OAuth client credentials, as IHE IUA's Get
-- Access Token asks); its role (a role's id) bounds what that token allows. Removing the row
-- revokes it.

CREATE TABLE clients (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    -- SHA-256 of the secret, hexadecimal; the secret is random, and is never stored
    secret_hash   TEXT NOT NULL,
    role          TEXT NOT NULL,
    -- the login of the user who made it
    created_by    TEXT NOT NULL,
    created_ms    BIGINT NOT NULL,
    -- when it last got an access token
    last_token_ms BIGINT
);
