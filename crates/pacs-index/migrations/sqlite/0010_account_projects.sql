-- Rights per project. An account is a user (`user:<id>`) or a client (`client:<id>`). One
-- that is in `limited_accounts` sees only the studies whose project is in its rows of
-- `account_projects`: none when it has no row, and never a study without a project. An
-- account that is not in `limited_accounts` sees everything.

CREATE TABLE limited_accounts (
    account TEXT PRIMARY KEY NOT NULL
);

CREATE TABLE account_projects (
    account TEXT NOT NULL,
    project TEXT NOT NULL,
    PRIMARY KEY (account, project)
);
