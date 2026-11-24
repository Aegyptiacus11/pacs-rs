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

## Roles and permissions

A user has one role, and a role gives permissions. No permission includes another: a role has
the ones it is given.

| Permission | Allows |
|---|---|
| `read` | studies, images, metadata, archives, searches of this archive (`/tools/find`, `/tools/create-archive`) |
| `write` | uploads, STOW-RS, measurements, hanging protocols |
| `remote` | everything about remote AEs (`/modalities`, `/jobs`) |
| `delete` | deleting patients, studies, series and instances |
| `users` | managing users and roles |

`remote` is apart from `read` because a search on a remote AE shows the hospital's patients,
identified, which someone who may only read a de-identified archive must not see.

A server starts with four roles, which can be changed or removed, and others added:

| Role | Permissions |
|---|---|
| `reader` | `read` |
| `editor` | `read`, `write`, `remote` |
| `manager` | `read`, `write`, `remote`, `delete` |
| `admin` | all of them, for good: this role cannot be changed or removed |

| Request | |
|---|---|
| `GET /roles` | every role: `id`, `name`, `permissions`, `locked`; for any signed-in user |
| `POST /roles` | `{"name", "permissions"}`: 201 with the role, whose `id` is made from the name; 409 when it exists |
| `PUT /roles/{id}` | `{"name"}`, `{"permissions"}` or both: the role after the change |
| `DELETE /roles/{id}` | 204; 409 while a user has the role |

Changing roles needs `users`. A change reaches the users who have the role at their next
refresh, within 15 minutes; `GET /auth/me` gives a user's `role` and `permissions`.

Open without a token: `/healthz` and `/readyz`, the web interface's own files (not `/ui/api/`), the three
sign-in routes below, `/auth/token` for [tools](#tools), and `OPTIONS` (CORS preflight). Everything else answers 401 without a
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
| `GET /auth/me` | `Authorization: Bearer` | `{"id", "login", "role", "permissions"}` |
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

## Signing in through a provider

With `PACS_OIDC_ISSUER` set, people sign in through the hospital's OpenID provider (Keycloak,
Microsoft Entra ID, Rauthy and the like), by the authorization code flow of OpenID Connect
Core 1.0 with PKCE. The provider is then **the only way in**: no password signs anyone in,
the first administrator's included, `POST /auth/login`, `POST /auth/password` and new users
with a password are refused, and so is `POST /auth/token` (tools get their tokens from the
provider too).

```bash
PACS_AUTH=true \
PACS_AUTH_SECRET="$(openssl rand -hex 32)" \
PACS_OIDC_ISSUER=https://idp.example.org/realms/hospital \
PACS_OIDC_CLIENT_ID=pacs \
PACS_OIDC_CLIENT_SECRET_FILE=/run/secrets/oidc-client \
PACS_OIDC_REDIRECT_URL=https://pacs.example.org/auth/oidc/callback \
PACS_OIDC_ADMINS=it-lead@example.org \
pacs-server
```

At the provider, the client is confidential (it has a secret), its redirect URI is
`PACS_OIDC_REDIRECT_URL`, and it may use the authorization code flow with PKCE (`S256`).

| Request | | |
|---|---|---|
| `GET /auth/methods` | | `{"password": false, "provider": true}`: what the sign-in page offers |
| `GET /auth/oidc/start?next=<page>` | | to the provider's sign-in page |
| `GET /auth/oidc/callback` | the provider's answer | to the web interface, signed in, with the refresh cookie |

- **Accounts.** Whoever the provider lets through gets an account at their first sign-in,
  with the role `PACS_OIDC_ROLE` (`reader`); an administrator changes it under
  [Users](#users) like any other, and a deactivated account is refused. An account is the
  provider's issuer and the person's subject identifier there, the only key OpenID Connect
  says is stable: a new email at the provider changes nothing. Its login is the email when
  the provider says it verified it, else the provider's user name.
- **Administrators.** The addresses of `PACS_OIDC_ADMINS` get the role `admin` at sign-in,
  only when the provider says it verified them. A verified email also links an account made
  before the provider was set, with the same login, so its role and projects stay.
- **What is checked.** The state against the cookie set at the start, then the code
  exchanged with the PKCE verifier and the client's secret, then the ID token: signed with a
  key the provider publishes (never a shared secret, never unsigned), its issuer the
  provider's, its audience this client (and `azp` this client when there are several), not
  expired, its nonce the one sent. Any failure lands on the sign-in page with
  `?signin_error=` and the reason (`expired`, `refused`, `provider`, `token`, `account`), and
  is in the [audit trail](audit.md) with the detail.
- **The provider's settings** come from its discovery document at start; a provider that
  cannot be reached, or names itself otherwise than `PACS_OIDC_ISSUER`, stops the start. Its
  keys are fetched again when a token names one not seen yet, so a key rotation needs no
  restart. Its URLs are HTTPS, or plain HTTP to `localhost` only.
- **Tools.** With a provider, a tool gets its access tokens there, not from `POST /auth/token`
  (see below).
- After the callback, the session is the one a password would give: the refresh cookie,
  then access tokens from `POST /auth/refresh`. Signing out ends it here, not at the
  provider.

### Tools through the provider

A tool is a client at the provider (client credentials), and is registered here under that
client id: `POST /clients` with `{"client_id": "<its id there>", "name", "role"}`, or
Settings > Tools. It has no secret here. Its access tokens are accepted when:

- a JWT signed with a key the provider publishes (RFC 9068), or, for any other token, one the
  provider's introspection endpoint calls active (RFC 7662, IUA ITI-102), asked with this
  server's client credentials;
- issued by the provider, not expired, and **for this server**: its audience is
  `PACS_OIDC_AUDIENCE`, by default the origin of `PACS_OIDC_REDIRECT_URL` (for example
  `https://pacs.example.org`). The tool asks for it as `resource` (RFC 8707), which the
  provider must allow for that client; a token issued for the tool itself is refused;
- for a tool registered here, named by its `client_id` (or `azp`, for a provider that does
  not send `client_id`).

What the tool may do is its role here. The token's scopes `pacs:read`, `pacs:write`,
`pacs:remote` and `pacs:delete` narrow it, never widen it: a token without any `pacs:` scope
gets the whole role. A tool never manages users. A refused token is answered 401, with
`error="invalid_token"` or `error="insufficient_scope"`.

## Users

For a role with `users`.

| Request | |
|---|---|
| `GET /users` | every user: `id`, `login`, `role`, `active`, `created_ms`, `projects` |
| `POST /users` | `{"login", "password", "role"}`, and `"projects"` to [limit them](#rights-per-project): 201 with the user; 409 when the login is taken |
| `PUT /users/{id}` | any of `{"password", "role", "active", "projects"}`: the user after the change |
| `DELETE /users/{id}` | 204 |

- A password is 12 to 1024 bytes, stored as an argon2id hash.
- A new password, a deactivation or a removal ends the user's sessions at once. Their access
  token, and a new role, last until that token runs out: 15 minutes at most.
- The last active user with the `admin` role cannot be removed, deactivated or given another
  role (409).

## Tools

A program that connects without a person (a viewer, a script, a pipeline) is a client: it has
an ID, a secret and a role, and exchanges the first two for a short access token. This is the
OAuth client credentials grant as IHE's Internet User Authorization profile (IUA) uses it:
Get Access Token [ITI-71] to get the token, Incorporate Access Token [ITI-72] to send it.

An administrator adds a tool in the web interface (Settings, Tools) or with `POST /clients`.
The secret is in that answer and nowhere after: the index keeps its SHA-256.

```bash
# once, by an administrator: the answer holds "id" and "secret"
curl -H "Authorization: Bearer <an administrator's access token>" -H 'content-type: application/json' \
  -d '{"name":"Nightly pipeline","role":"editor"}' https://pacs.example/clients

# the tool, whenever its token has run out
curl -u "$ID:$SECRET" -d grant_type=client_credentials https://pacs.example/auth/token
curl -H "Authorization: Bearer <access_token>" https://pacs.example/dicom-web/studies
```

`POST /auth/token` takes a form (`application/x-www-form-urlencoded`), with the ID and the
secret in `Authorization: Basic`:

| Parameter | |
|---|---|
| `grant_type` | `client_credentials` |
| `scope` | optional: permissions separated by spaces, to get less than the role gives |
| `resource` | optional: this server's name (`PACS_AUTH_ISSUER`); another one is refused |
| `requested_token_type` | optional: `urn:ietf:params:oauth:token-type:jwt`, the only kind |

The answer is `{"access_token", "token_type": "Bearer", "scope", "expires_in": 300}`, never
kept by a cache. A request that is refused gets OAuth's errors as `{"error",
"error_description"}`: 401 `invalid_client` for an unknown ID or a wrong secret, 400
`invalid_request`, `unsupported_grant_type`, `invalid_scope` or `invalid_target` for the rest.

- **The access token** is a JWT signed with HS256, good for five minutes, with the claims
  IUA's JSON Web Token option asks for: `iss` and `aud` (both `PACS_AUTH_ISSUER`), `sub` and
  `client_id` (the tool's ID), `jti`, `iat`, `exp` and `scope`. It goes in
  `Authorization: Bearer`, never in a URL.
- **What it allows** is its scope, which is at most the role's `read`, `write`, `remote` and
  `delete`. A tool never has `users`, whatever its role: nothing of the users, the roles, the
  tools or `/auth`.
- **Every request is checked against the tool as it is now.** A tool that is removed, a role
  it is moved to, a permission taken from its role: each holds at the tool's next request,
  for the tokens it already has too.
- **A refused request gets 401**, as ITI-72 asks, not the 403 a user gets: with
  `WWW-Authenticate: Bearer error="invalid_token"` when the token is not good, and
  `error="insufficient_scope"` when it does not allow the request.

| Request | |
|---|---|
| `GET /clients` | every tool: `id`, `name`, `role`, `created_by`, `created_ms`, `last_token_ms`, `projects` |
| `POST /clients` | `{"name", "role"}`, and `"projects"` to [limit it](#rights-per-project): 201 with the tool and its `secret`, this once |
| `PUT /clients/{id}` | any of `{"name", "role", "projects"}`: the tool after the change |
| `DELETE /clients/{id}` | 204: the tool is revoked |

These need `users`. A role cannot be removed while a tool has it (409).

What this is not: a complete IUA Authorization Server. There is no authorization code grant
(a person signs in with a password here), no server metadata [ITI-103] and no token
introspection [ITI-102]; the tokens are signed with the server's own secret, so only pacs-rs
can check them; and TLS, which IUA requires, is the proxy's in front of pacs-rs.

## Rights per project

The role says what an account may do. Its projects say on which studies. A user or a tool
sees every study, unless it is limited to some of the server's
[projects](de-identification.md): then the rest of the archive is, to that account, not
there.

| `projects` of a user or a tool | It sees |
|---|---|
| `null`, or left out | every study |
| `["liver-2026"]` | the studies that came in through that project |
| `[]` | no study |

- **A study in no project** (stored as received, so identified) is seen only by an account
  that sees every study. Someone limited to de-identified projects never sees an identified
  study.
- **Outside its projects an account is answered 404**, as for a study that does not exist,
  on every route: the REST API, DICOMweb, the web interface's lists, archives, searches and
  download links. Lists and counts hold only what it sees. A patient is the studies of
  theirs that it sees.
- **The attempt is recorded.** The answer is 404, and the [audit
  trail](audit.md#what-is-recorded) names the account and the study it asked for.
- **Writing.** Measurements go on the studies it sees. Deleting takes only what it sees: of
  a patient, the studies in its projects, and the others stay. Over HTTP it stores only
  [results](de-identification.md) for its projects' studies: anything else would be in no
  project, out of its own sight (403 for an upload, the instance's failure in a STOW-RS
  answer). A study comes into a project through the project's AE title.
- **Jobs.** `/jobs` lists, shows and cancels the moves into its projects and the sendings of
  studies it sees; any other job is answered 404.
- **Remote archives.** With the `remote` permission it may search them, and bring studies in
  only with `"Project"` naming one of its own (403 otherwise). A search there shows the
  hospital's patients, identified: `remote` is not for an account that must not see them.
- **Setting it.** In the web interface (Settings, Users or Tools: *Studies they see*), or
  with `"projects"` in `POST` and `PUT` of `/users` and `/clients`. A name that is not a
  project of this server is refused (422). `GET /projects` lists the names, for who manages
  users. `GET /auth/me` tells an account its own.
- **A change holds from the account's next request**, tokens it already has included: the
  limit is read from the index each time, not from the token.
- **Who is not limited**: the first administrator, and every account until someone limits
  it. An administrator can limit another, and nobody changes their own unless they manage
  users.

What this does not cover: DIMSE has no accounts, so whoever may open an association to a
project's AE title may send to it. Searching and retrieving over DIMSE are bounded by the
AE title that is called instead: a project's shows, and sends, that project's studies and
no other, and only the AE titles the server is configured with are answered ([being
searched](dimse.md#being-searched), [being retrieved from](dimse.md#being-retrieved-from)).

## Not yet

- Sending the [audit trail](audit.md) to a repository.
