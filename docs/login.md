# Login

Off by default: without it, anyone who reaches the server can read and delete everything.
`PACS_AUTH=true` puts a login before every HTTP route.

```bash
PACS_AUTH=true \
PACS_AUTH_SECRET="$(openssl rand -hex 32)" \
PACS_ADMIN_PASSWORD='a first password, 12 bytes or more' \
pacs-server
```

- `PACS_AUTH_SECRET` (or `PACS_AUTH_SECRET_FILE`) signs the access tokens: at least 32 bytes,
  the same on every start. Changing it refuses every access token already given out; sessions
  go on at their next refresh.
- `PACS_ADMIN_PASSWORD` (or `..._FILE`) is read only while the index has no user: the first
  start makes the administrator `PACS_ADMIN_LOGIN` (`admin`) with it. After that it can be
  removed, and changing it changes nothing.
- A start with the login on and something missing (the secret, a first password of 12 bytes
  or more) stops with the reason; it never starts unguarded.

The [web interface](web-interface.md) then opens on a sign-in page, and `/ui/config.json` says
`"auth": true`.

DIMSE has no login here: `PACS_DIMSE_KNOWN_CALLERS_ONLY` and the network decide who may open
an association.

## Roles

Each role includes the ones before it.

| Role | May |
|---|---|
| `reader` | read: studies, images, metadata, archives, searches of this archive (`/tools/find`, `/tools/create-archive`) |
| `editor` | also write: uploads, STOW-RS, measurements, hanging protocols, and everything about remote AEs (`/modalities`, `/jobs`) |
| `manager` | also delete patients, studies, series and instances |
| `admin` | also manage users |

Remote AEs need `editor` even to list or search: a search there shows the hospital's
patients, identified, which someone who may only read a de-identified archive must not see.

Open without a token: `/healthz`, the web interface's own files (not `/ui/api/`), the three
sign-in routes below, and `OPTIONS` (CORS preflight). Everything else answers 401 without a
valid token, a route that does not exist included, and 403 with a role that does not allow it.
Errors are `{"detail": "..."}`; a 401 carries `WWW-Authenticate: Bearer`.

## Signing in

Sessions are those of the [jwt-sessions](https://github.com/Aegyptiacus11/jwt-sessions-rs)
crate.

| Request | | |
|---|---|---|
| `POST /auth/login` | `{"email": "<login>", "password": "..."}` | `{"access_token", "token_type"}` and the refresh cookie |
| `POST /auth/refresh` | the refresh cookie | a new access token and the rotated cookie |
| `POST /auth/logout` | the refresh cookie | 204; that session ends |
| `POST /auth/logout-all` | `Authorization: Bearer` | 204; every session of the user ends |
| `GET /auth/me` | `Authorization: Bearer` | `{"id", "login", "role"}` |
| `POST /auth/password` | `{"current_password", "new_password"}` | 204; the password is changed and every session of the user ends |

- **The access token** is a JWT valid for 15 minutes, sent as `Authorization: Bearer <token>`
  on every request. It is never in a cookie, so no request is authorised by a cookie alone
  and no route needs protection against forged cross-site requests.
- **The refresh token** travels only in an `HttpOnly`, `SameSite=Lax` cookie that the browser
  sends to `/auth/*` alone. Each refresh spends it and sets its successor; a spent one
  presented again ends the session, since two parties then hold it. Sessions last 30 days
  from their last refresh and outlive a restart: they are in the index (only the SHA-256 of
  each token).
- A wrong password and an unknown login get the same answer, after the same work.
- Logins ignore case.

A script signs in and keeps the cookie to refresh:

```bash
curl -c jar -H 'content-type: application/json' \
  -d '{"email":"admin","password":"..."}' https://pacs.example/auth/login
curl -H "Authorization: Bearer <access_token>" https://pacs.example/dicom-web/studies
curl -b jar -c jar -X POST https://pacs.example/auth/refresh
```

### Links a browser follows itself

A browser cannot send the header on a link it follows: a download, a page opened in a tab.
For those:

| Request | | |
|---|---|---|
| `POST /auth/download-token` | `{"path": "/studies/<id>/archive"}` | `{"token", "expires_in"}` |

The token is good for one minute and for `GET` on that path alone, query included, sent as
`?token=`. The user must be allowed to read the path; there is none for `/users` or `/auth`.
It is the only token a URL may carry: an access token in a query is refused, and so is a
download token in the header. The request log leaves it out.

Behind a proxy that serves pacs-rs under a prefix, set `PACS_AUTH_COOKIE_PATH` to the path of
`/auth` as the browser sees it (`/pacs/auth`). The cookie is `Secure`: over plain HTTP on
localhost, `PACS_AUTH_COOKIE_SECURE=false` lets a browser keep it.

## Users

For the administrator.

| Request | |
|---|---|
| `GET /users` | every user: `id`, `login`, `role`, `active`, `created_ms` |
| `POST /users` | `{"login", "password", "role"}`: 201 with the user; 409 when the login is taken |
| `PUT /users/{id}` | any of `{"password", "role", "active"}`: the user after the change |
| `DELETE /users/{id}` | 204 |

- A password is 12 to 1024 bytes, stored as an argon2id hash.
- A new password, a deactivation or a removal ends the user's sessions at once. Their access
  token, and a new role, last until that token runs out: 15 minutes at most.
- The last active administrator cannot be removed, deactivated or given another role (409).

## Not yet

- Tokens for tools that cannot hold a session (a viewer, a pipeline), revocable one by one.
- Signing in through an identity provider (OpenID Connect).
- Rights per project.
- An audit trail of who read, stored and deleted what.
