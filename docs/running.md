# Running

With Docker Compose, from the published image (no build):

```bash
curl -fsSLO https://raw.githubusercontent.com/Aegyptiacus11/pacs-rs/main/compose.yaml
docker compose up -d --wait
```

The web interface is then at `http://127.0.0.1:8080/ui/` and DICOMweb at
`http://127.0.0.1:8080/dicom-web`; the index and files stay in the `pacs-data` volume. Settings go
in the file's `environment` (the table below). To stay on one version instead of `latest`, run
`PACS_IMAGE=ghcr.io/aegyptiacus11/pacs-rs:0.6.0 docker compose up -d`. The [login](login.md) is off by default, so the file publishes the port on 127.0.0.1 only: turn it on, or put an
authenticating proxy in front, before opening it to a network.

From source:

```bash
cargo run --release -p pacs-server -- --help
cargo run --release -p pacs-server            # files in data/files, index in data/index.sqlite
```

Every flag has a `PACS_*` environment variable:

| Variable | Default | |
|---|---|---|
| `PACS_LISTEN` | `127.0.0.1:8080` | address to listen on |
| `PACS_DICOMWEB_PATH` | `/dicom-web` | where the DICOMweb root is served |
| `PACS_BASE_URL` | from the request | public URL of the DICOMweb root, for Retrieve URLs |
| `PACS_INDEX` | `data/index.sqlite` | a SQLite file, or `postgres://user@host/database` |
| `PACS_INDEX_PASSWORD` | | the Postgres password; environment only, or `..._FILE` naming a file |
| `PACS_STORAGE` | `file:data/files` | `file:<directory>` or `s3://<bucket>[/<prefix>]` |
| `PACS_S3_ENDPOINT` | AWS | e.g. `http://localhost:8333` for SeaweedFS |
| `PACS_S3_REGION` | `us-east-1` | |
| `PACS_S3_ACCESS_KEY_ID`, `PACS_S3_SECRET_ACCESS_KEY` | | environment only, or `..._FILE` naming a file |
| `PACS_OVERWRITE_INSTANCES` | `false` | replace an instance sent again, instead of keeping the first copy |
| `PACS_COMPRESSION` | `jpeg-ls` | how uncompressed pixel data is stored: `jpeg-ls` (lossless, checked) or `off` |
| `PACS_MAX_UPLOAD_BYTES` | 1 GiB | largest STOW-RS or upload body |
| `PACS_MAX_READ_BYTES` | 512 MiB | memory that reading files may take at once beyond the files themselves, by estimate (see [costly files](dicomweb.md#elements-in-un-wrong-lengths-and-costly-files)) |
| `PACS_CORS_ORIGINS` | none | comma-separated origins, or `*` |
| `PACS_UI` | `true` | serve the web interface at `/ui/` |
| `PACS_AUTH` | `false` | ask for a [login](login.md) on every HTTP route but `/healthz` and the web interface's files |
| `PACS_AUTH_SECRET` | | the secret that signs access tokens, at least 32 bytes; environment only, or `..._FILE` naming a file |
| `PACS_ADMIN_LOGIN` | `admin` | the first administrator's login, made when there is no user yet |
| `PACS_ADMIN_PASSWORD` | | the first administrator's password, 12 bytes or more, read only when there is no user yet; environment only, or `..._FILE` |
| `PACS_AUTH_COOKIE_SECURE` | `true` | send the refresh cookie over HTTPS only |
| `PACS_AUTH_COOKIE_PATH` | `/auth` | the cookie's path as the browser sees it, behind a proxy that adds a prefix |
| `PACS_DIMSE_LISTEN` | off | address for DIMSE, e.g. `0.0.0.0:11112` |
| `PACS_DIMSE_AE_TITLE` | `PACSRS` | pacs-rs's AE title, called and calling |
| `PACS_DIMSE_EXTRA_AE_TITLES` | none | further called AE titles accepted, comma-separated |
| `PACS_DIMSE_REMOTES` | none | remote AEs, comma-separated, each `id=AE_TITLE@host:port` |
| `PACS_DIMSE_KNOWN_CALLERS_ONLY` | `false` | accept associations only from the remotes' AE titles |
| `PACS_DIMSE_MAX_ASSOCIATIONS` | `16` | associations served at once |
| `PACS_DIMSE_TIMEOUT` | `30` | seconds to wait for a remote AE |
| `PACS_DIMSE_MOVE_TIMEOUT` | `600` | seconds to wait between two C-MOVE responses, while a large study is sent |
| `PACS_PROJECTS` | none | a JSON file of projects, each an AE title and a de-identification profile (see [De-identification](de-identification.md)) |
| `PACS_PROJECT_SECRET_<AE title>` | | a project's secret; environment only, or `..._FILE` naming a file |
| `PACS_QUARANTINE` | none | `file:<directory>` or `s3://<bucket>[/<prefix>]` for quarantined instances; without it they are refused |
| `RUST_LOG` | `info` | log filter |

The image is `ghcr.io/aegyptiacus11/pacs-rs` (linux/amd64 and linux/arm64; `:latest` or a
version such as `:0.6.0`), or build it with `docker build -t pacs-rs .`: a static binary on an
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
