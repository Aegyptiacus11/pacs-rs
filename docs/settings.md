# Settings

Every setting of `pacs-server`, by what it is about. Each is an environment variable, and a
flag of the same name (`PACS_MAX_REQUESTS` is `--max-requests`; `pacs-server --help` lists
them). A test compares this page with the program: a setting it has and this page lacks, or
a default that differs, fails the build.

**Secrets come from the environment only, never from a flag**, which other users of the
machine could read: the Postgres password, the S3 keys, the signing secret, the first
administrator's password and the projects' secrets. Each can also be given as a file, by
the variable with `_FILE` appended naming it (`PACS_AUTH_SECRET_FILE=/run/secrets/auth`),
which is how Docker and Kubernetes hand secrets over.

A start whose settings do not add up stops with the reason, rather than running with less
than was asked.

## Addresses

Where the server listens and how it names itself.

| Variable | Default | |
|---|---|---|
| `PACS_LISTEN` | `127.0.0.1:8080` | address to listen on |
| `PACS_DICOMWEB_PATH` | `/dicom-web` | where the DICOMweb root is served |
| `PACS_BASE_URL` | from the request | public URL of the DICOMweb root, for Retrieve URLs |
| `PACS_CORS_ORIGINS` | none | comma-separated origins, or `*` |
| `PACS_UI` | `true` | serve the web interface at `/ui/` |

## Files and index

Where the files and the index are kept. See [Running](running.md) for Docker, and [TLS to Postgres](running.md#tls-to-postgres).

| Variable | Default | |
|---|---|---|
| `PACS_STORAGE` | `file:data/files` | `file:<directory>` or `s3://<bucket>[/<prefix>]` |
| `PACS_S3_ENDPOINT` | AWS | e.g. `http://localhost:8333` for SeaweedFS |
| `PACS_S3_REGION` | `us-east-1` | |
| `PACS_S3_ACCESS_KEY_ID`, `PACS_S3_SECRET_ACCESS_KEY` | | environment only, or `..._FILE` naming a file |
| `PACS_INDEX` | `data/index.sqlite` | a SQLite file, or `postgres://user@host/database` |
| `PACS_INDEX_PASSWORD` | | the Postgres password; environment only, or `..._FILE` naming a file |
| `PACS_INDEX_TLS` | `prefer` | [TLS to a Postgres index](running.md#tls-to-postgres): `prefer`, `verify-full` or `off` |
| `PACS_INDEX_CA_FILE` | the web's authorities | the certificate authority (PEM) that signed the Postgres server's certificate, for `verify-full` |

## What is stored, and how

| Variable | Default | |
|---|---|---|
| `PACS_OVERWRITE_INSTANCES` | `false` | replace an instance sent again, instead of keeping the first copy |
| `PACS_COMPRESSION` | `jpeg-ls` | how uncompressed pixel data is stored: `jpeg-ls` (lossless, checked) or `off` |
| `PACS_MAX_UPLOAD_BYTES` | 1 GiB | largest STOW-RS or upload body |
| `PACS_CHECK_FILES_DAYS` | `30` | check a stored file against its SHA-256 again once its last check is this many days old, in the background; `0`: never (see [Checking stored files](integrity.md)) |
| `PACS_MAX_READ_BYTES` | 512 MiB | memory that reading files may take at once beyond the files themselves, by estimate (see [costly files](dicomweb.md#elements-in-un-wrong-lengths-and-costly-files)) |

## Login

See [Login](login.md).

| Variable | Default | |
|---|---|---|
| `PACS_AUTH` | `false` | ask for a [login](login.md) on every HTTP route but `/healthz` and the web interface's files |
| `PACS_AUTH_SECRET` | | the secret that signs access tokens, at least 32 bytes; environment only, or `..._FILE` naming a file |
| `PACS_ADMIN_LOGIN` | `admin` | the first administrator's login, made when there is no user yet |
| `PACS_ADMIN_PASSWORD` | | the first administrator's password, 12 bytes or more, read only when there is no user yet; environment only, or `..._FILE` |
| `PACS_AUTH_COOKIE_SECURE` | `true` | send the refresh cookie over HTTPS only |
| `PACS_AUTH_COOKIE_PATH` | the path of `/auth` | the cookie's path as the browser sees it, behind a proxy that adds a prefix |
| `PACS_AUTH_ISSUER` | `pacs-rs` | the name this server gives itself in the access tokens of [tools](login.md#tools): their `iss` and `aud` |
| `PACS_OIDC_ISSUER` | | sign people in through this [OpenID provider](login.md#signing-in-through-a-provider) instead of with passwords: its issuer, exactly as its discovery document names it |
| `PACS_OIDC_CLIENT_ID` | | this server's client id at the provider |
| `PACS_OIDC_CLIENT_SECRET` | | the client's secret at the provider; environment only, or `..._FILE` |
| `PACS_OIDC_REDIRECT_URL` | | this server's `/auth/oidc/callback` as the browser reaches it, registered at the provider |
| `PACS_OIDC_SCOPES` | `openid email profile` | the scopes asked for at sign-in, `openid` among them |
| `PACS_OIDC_AUDIENCE` | the origin of `PACS_OIDC_REDIRECT_URL` | what a tool's access token from the provider must name as its audience, and the tool asks for as `resource` |
| `PACS_OIDC_ROLE` | `reader` | the role of an account made at its first sign-in |
| `PACS_OIDC_ADMINS` | | email addresses, comma-separated, that get the role `admin` when the provider says it verified them |

## Audit trail

See [Audit trail](audit.md).

| Variable | Default | |
|---|---|---|
| `PACS_AUDIT_FILE` | none | keep an [audit trail](audit.md) in this file, one line per event |
| `PACS_AUDIT_SOURCE_ID` | `pacs-rs` | the name this server reports under in its audit messages |
| `PACS_AUDIT_REPOSITORY` | none | [send the audit trail](audit.md#sending-to-a-repository) to this repository over TLS: `host` or `host:port` (6514) |
| `PACS_AUDIT_REPOSITORY_CA_FILE` | | the authorities (PEM) that sign the repository's certificate; needed with it |
| `PACS_AUDIT_REPOSITORY_CERT_FILE`, `PACS_AUDIT_REPOSITORY_KEY_FILE` | none | the certificate and key (PEM) pacs-rs shows the repository |

## Limits on requests

See [Limits on requests](running.md#limits-on-requests).

| Variable | Default | |
|---|---|---|
| `PACS_HEADER_TIMEOUT` | `30` | seconds a client has to send a request's head; an idle connection is closed after it. `0`: no [limit](running.md#limits-on-requests) |
| `PACS_BODY_TIMEOUT` | `60` | seconds a request's body may pause |
| `PACS_REQUEST_TIMEOUT` | `300` | seconds within which a request without a body must be answered |
| `PACS_WRITE_TIMEOUT` | `60` | seconds a write to a client may stall |
| `PACS_MAX_CONNECTIONS` | `1024` | connections open at once. `0`: no bound |
| `PACS_MAX_REQUESTS` | `256` | requests served at once |
| `PACS_MAX_REQUESTS_PER_CLIENT` | `64` | requests served at once for one address |

## DIMSE

The DICOM network protocol: see [DIMSE](dimse.md), and [DIMSE over TLS](dimse.md#dimse-over-tls).

| Variable | Default | |
|---|---|---|
| `PACS_DIMSE_LISTEN` | off | address for DIMSE, e.g. `0.0.0.0:11112` |
| `PACS_DIMSE_AE_TITLE` | `PACSRS` | pacs-rs's AE title, called and calling |
| `PACS_DIMSE_EXTRA_AE_TITLES` | none | further called AE titles accepted, comma-separated |
| `PACS_DIMSE_REMOTES` | none | remote AEs, comma-separated, each `id=AE_TITLE@host:port`, with `/tls` after the port for one reached [over TLS](dimse.md#dimse-over-tls); they are also the AE titles whose [searches](dimse.md#being-searched) are answered |
| `PACS_DIMSE_MOVE_DESTINATIONS` | none | the remote AEs studies may be sent to, by their `id`, comma-separated: the destinations a C-MOVE asked of pacs-rs may name. An archive pacs-rs only reads from is left out |
| `PACS_DIMSE_KNOWN_CALLERS_ONLY` | `false` | accept associations only from the remotes' AE titles |
| `PACS_DIMSE_MAX_ASSOCIATIONS` | `16` | associations served at once |
| `PACS_DIMSE_TIMEOUT` | `30` | seconds to wait for a remote AE |
| `PACS_DIMSE_MOVE_TIMEOUT` | `600` | seconds to wait between two C-MOVE responses, while a large study is sent |
| `PACS_DIMSE_TLS_CERT_FILE`, `PACS_DIMSE_TLS_KEY_FILE` | none | pacs-rs's certificate (with its chain) and private key, PEM: with them the DIMSE port speaks TLS only |
| `PACS_DIMSE_TLS_CA_FILE` | none | the authorities (PEM) that sign the certificates of DIMSE callers and of the remote AEs called over TLS |
| `PACS_DIMSE_TLS_CLIENT_AUTH` | `required` | whether a DIMSE caller must show a certificate; `off` takes any caller that speaks TLS |

## Projects

De-identification on the way in: see [De-identification](de-identification.md).

| Variable | Default | |
|---|---|---|
| `PACS_PROJECTS` | none | a JSON file of projects, each an AE title and a de-identification profile (see [De-identification](de-identification.md)) |
| `PACS_PROJECT_SECRET_<AE title>` | | a project's secret; environment only, or `..._FILE` naming a file |
| `PACS_QUARANTINE` | none | `file:<directory>` or `s3://<bucket>[/<prefix>]` for quarantined instances; without it they are refused |

## Logging

| Variable | Default | |
|---|---|---|
| `RUST_LOG` | `info` | log filter |
