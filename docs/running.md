# Running

With Docker Compose, from the published image (no build):

```bash
curl -fsSLO https://raw.githubusercontent.com/Aegyptiacus11/pacs-rs/main/compose.yaml
docker compose up -d --wait
```

The web interface is then at `http://127.0.0.1:8080/ui/` and DICOMweb at
`http://127.0.0.1:8080/dicom-web`; the index and files stay in the `pacs-data` volume. Settings go
in the file's `environment` (the table below). To stay on one version instead of `latest`, run
`PACS_IMAGE=ghcr.io/aegyptiacus11/pacs-rs:0.9.0 docker compose up -d`. The [login](login.md) is off by default, so the file publishes the port on 127.0.0.1 only: turn it on, or put an
authenticating proxy in front, before opening it to a network.

From source:

```bash
cargo run --release -p pacs-server -- --help
cargo run --release -p pacs-server            # files in data/files, index in data/index.sqlite
```

Every setting is a `PACS_*` environment variable, and a flag: they are all on
[Settings](settings.md), by what they are about.

The image is `ghcr.io/aegyptiacus11/pacs-rs` (linux/amd64 and linux/arm64; `:latest` or a
version such as `:0.9.0`), or build it with `docker build -t pacs-rs .`: a static binary on an
empty image, about 40 MB with the web interface, run as
an unprivileged user (uid 10001). It listens on `0.0.0.0:8080` and keeps the index and files in
the `/data` volume unless `PACS_STORAGE` says otherwise. There is no shell or curl inside, so the
image's health check is the binary itself: `pacs-server --healthcheck` asks `/healthz` on the
address of `PACS_LISTEN` and exits 0 or 1. For DIMSE, set `PACS_DIMSE_LISTEN=0.0.0.0:11112` and
publish that port.

A synthetic series to try it with:

```bash
cargo run -p pacs-core --features synthetic --example write_synthetic -- synthetic 1 3
curl -X POST --data-binary @synthetic/00001.dcm http://127.0.0.1:8080/instances
```

## TLS to Postgres

The index holds patient names and identifiers, and the connection to Postgres carries them.

| `PACS_INDEX_TLS` | The connection | Who the server is |
|---|---|---|
| `prefer`, the default | encrypted when the server offers TLS, plain when it does not | not checked |
| `verify-full` | encrypted, or the start fails | its certificate must chain to an authority pacs-rs knows and name the host in `PACS_INDEX` |
| `off` | plain | not checked |

- **Use `verify-full` whenever Postgres is on another machine.** `prefer` keeps a passive
  listener out, but not someone who can answer in the server's place.
- `PACS_INDEX_CA_FILE` names the authority that signed the server's certificate, as a PEM
  file. Without it the web's public authorities are used, which is right for a managed
  database with a public certificate and wrong for one you signed yourself.
- The certificate must name what `PACS_INDEX` connects to: a server reached as `db.internal`
  needs `db.internal` in its certificate, and one reached by address needs the address.
- A start that cannot have what it was asked for stops with the reason: a certificate that
  does not check out, a server without TLS under `verify-full`, a file that cannot be read,
  or these settings with a SQLite index. Set the TLS here, not as `sslmode` in the URL.

## Limits on requests

So that one client cannot hold the server, each part of a request has a time, and the
requests served at once have a number.

| Limit | Setting | Over it |
|---|---|---|
| The request's head must arrive within | `PACS_HEADER_TIMEOUT` | the connection is closed |
| The request's body may pause for at most | `PACS_BODY_TIMEOUT` | the request fails, the connection is closed |
| A request without a body must be answered within | `PACS_REQUEST_TIMEOUT` | 503 |
| A write to the client may stall for at most | `PACS_WRITE_TIMEOUT` | the connection is closed |
| Connections open at once | `PACS_MAX_CONNECTIONS` | further ones wait to be accepted |
| Requests being served at once | `PACS_MAX_REQUESTS` | 503, `Retry-After: 1` |
| Requests being served at once for one address | `PACS_MAX_REQUESTS_PER_CLIENT` | 429, `Retry-After: 1` |

- The timeouts are about pauses, not sizes: an upload or a download takes as long as it needs
  while bytes keep moving.
- The head's timeout runs from the connection's opening and again from each answer, so a
  connection kept open and left idle is closed after it.
- `PACS_REQUEST_TIMEOUT` is for `GET`, `HEAD`, `DELETE` and `OPTIONS`. A request with a body
  is bounded by the pauses of its body, and a synchronous C-MOVE
  (`POST /modalities/{id}/move`) by `PACS_DIMSE_MOVE_TIMEOUT`.
- A request counts as being served until its answer's body has been sent: a download, to its
  last byte.
- `/healthz` is outside the two bounds on requests: a busy server still says it is up.
- **Behind a reverse proxy** every request comes from the proxy's address, so
  `PACS_MAX_REQUESTS_PER_CLIENT` bounds all its users together: raise it, or set it to `0` and
  bound clients at the proxy.

DIMSE has its own: `PACS_DIMSE_TIMEOUT`, `PACS_DIMSE_MOVE_TIMEOUT` and
`PACS_DIMSE_MAX_ASSOCIATIONS`.
