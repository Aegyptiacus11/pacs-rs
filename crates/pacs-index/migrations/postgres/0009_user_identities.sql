-- Who a user is at an OpenID provider (PACS_OIDC_ISSUER): the provider's issuer and its
-- subject identifier, which OpenID Connect Core 1.0 section 5.7 names as the only stable
-- key for an end user. A user made at their first sign-in through the provider has one.

CREATE TABLE user_identities (
    issuer  TEXT NOT NULL,
    subject TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (issuer, subject)
);

CREATE INDEX ix_user_identities_user_id ON user_identities (user_id);
