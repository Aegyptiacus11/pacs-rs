# Running

With Docker Compose, from the published image (no build):

```bash
curl -fsSLO https://raw.githubusercontent.com/Aegyptiacus11/pacs-rs/main/compose.yaml
docker compose up -d --wait
```

The web interface is then at `http://127.0.0.1:8080/ui/` and DICOMweb at
`http://127.0.0.1:8080/dicom-web`; the index and files stay in the `pacs-data` volume. Settings go
in the file's `environment` (the table below). To stay on one version instead of `latest`, run
`PACS_IMAGE=ghcr.io/aegyptiacus11/pacs-rs:0.16.3 docker compose up -d`. The [login](login.md) is off by default, so the file publishes the port on 127.0.0.1 only: turn it on, or put an
authenticating proxy in front, before opening it to a network.

From source:

```bash
cargo run --release -p pacs-server -- --help
cargo run --release -p pacs-server            # files in data/files, index in data/index.sqlite
```

Every setting is a `PACS_*` environment variable, and a flag: they are all on
[Settings](settings.md), by what they are about.

The image is `ghcr.io/aegyptiacus11/pacs-rs` (linux/amd64 and linux/arm64; `:latest` or a
version such as `:0.16.3`), or build it with `docker build -t pacs-rs .`: a static binary on an
empty image, about 40 MB with the web interface, run as
an unprivileged user (uid 10001). It listens on `0.0.0.0:8080` and keeps the index and files in
the `/data` volume unless `PACS_STORAGE` says otherwise. There is no shell or curl inside, so the
image's health check is the binary itself: `pacs-server --healthcheck` asks `/healthz` on the
address of `PACS_LISTEN` and exits 0 or 1. For DIMSE, set `PACS_DIMSE_LISTEN=0.0.0.0:11112` and
publish that port.

Two checks, both open without a token and answered even when the server is at its
[limits](#limits-on-requests):

| | Answers 200 when | For |
|---|---|---|
| `GET /healthz` | the process is up | restarting a process that hangs (the image's health check) |
| `GET /readyz` | the index and the store both answer, within 5 seconds each | a load balancer or an orchestrator sending requests only to a server that can serve them |

`/readyz` answers `{"index": "ok", "store": "ok"}`, or 503 with the error of the part that does
not answer in its place. It is not the image's health check on purpose: a database down for a
minute should take the server out of rotation, not have it restarted.

A study to try it with, the [sample](sample-study.md) of the quickstart:

```bash
curl -X POST --data-binary @docs/sample/00001.dcm http://127.0.0.1:8080/instances
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

## Several servers

Two or more pacs-rs can serve the same archive: one Postgres index (`PACS_INDEX`) and one
store (`PACS_STORAGE`, a directory they share or the same S3 bucket), behind a load balancer.
Uploads and deletions stay correct across them (`crates/pacs-server/tests/several_servers.rs`
races them on two servers): the index serializes its writes with a Postgres advisory lock,
and each stored file has a key of its own, so no server removes a file another just stored.

- **Moves in the background** run on the server that started them; each shows it is alive
  every quarter of a second, and any server takes up one whose server went silent for 30
  seconds, after a crash or a restart (see [DIMSE](dimse.md)).
- **The scheduled tasks** (file checks, emptying the trash) may run on every server: what
  they do twice does no harm.
- **SQLite is one server's.** Its file cannot be shared between hosts; for several servers,
  use Postgres.
- **The audit trail** is each server's own file (`PACS_AUDIT_FILE`): give each server its
  own `PACS_AUDIT_SOURCE_ID`, and send them all to one
  [repository](audit.md#sending-to-a-repository) (`PACS_AUDIT_REPOSITORY`).
- Sessions of the [login](login.md) are in the index: a person signed in on one server is
  signed in on all of them, given the same `PACS_AUTH_SECRET`.

## Metrics

With `PACS_METRICS_LISTEN` (e.g. `127.0.0.1:9100`), the server serves `GET /metrics` in the
Prometheus text format on that address, and nothing else there. It is apart from the
address users reach, so a scraper needs no login, and the metrics are never public by
chance: keep it on a private network. Off by default.

| Series | Kind | |
|---|---|---|
| `pacs_http_requests_total{method, status}` | counter | HTTP requests answered, by method and status class (`2xx`, `4xx`...) |
| `pacs_http_request_seconds_total{method, status}` | counter | the time spent answering them |
| `pacs_dimse_associations_open` | gauge | DIMSE associations being served |
| `pacs_dimse_associations_total{outcome}` | counter | connections that became an association (`accepted`) or did not (`refused`) |
| `pacs_instances_stored_total{outcome}` | counter | instances received: `added`, `replaced`, `already_stored` |
| `pacs_errors_total{crate}` | counter | lines logged at ERROR, by the crate that logged them |
| `pacs_studies`, `pacs_instances` | gauge | what the archive holds |
| `pacs_storage_bytes` | gauge | the bytes of the files the index names (the trash aside) |
| `pacs_files_failed`, `pacs_files_never_checked` | gauge | [file checks](integrity.md) that failed, files not checked yet |
| `pacs_trash_entries` | gauge | deletions waiting in the [trash](trash.md) |
| `pacs_build_info{version}`, `pacs_uptime_seconds` | gauge | the version running, and since when |

The counters start at 0 with the process; the gauges of the archive are read from the index
at each scrape, which answers 503 when the index does not. With
[several servers](#several-servers), scrape each: the archive's gauges are the same on all.

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
