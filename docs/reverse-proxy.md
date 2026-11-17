# Behind a reverse proxy

pacs-rs speaks plain HTTP. On a network, put a reverse proxy in front of it that speaks
HTTPS to the world: nginx, Caddy, Traefik or a hospital's own. This page says what the
proxy must do and what pacs-rs must be told. The nginx settings below were tried against a
server while the page was written.

## What the proxy must do

| Need | Why | In nginx |
|---|---|---|
| Speak HTTPS | the login's cookie is sent over HTTPS only, and tokens and images must not travel in clear | `listen 443 ssl;` with your certificate |
| Say what the visitor asked for | the addresses pacs-rs gives out (the Retrieve URLs of DICOMweb) are built from it | `X-Forwarded-Proto`, `X-Forwarded-Host` |
| Let large bodies through | a study uploaded in one request is far over a megabyte | `client_max_body_size 0;` |
| Not hold bodies back | uploads and downloads are streamed, and their timeouts count pauses | `proxy_request_buffering off; proxy_buffering off;` |
| Wait as long as pacs-rs does | a synchronous C-MOVE answers when the study has arrived | `proxy_read_timeout` over `PACS_DIMSE_MOVE_TIMEOUT` |

```nginx
server {
    listen 443 ssl;
    server_name pacs.example;
    ssl_certificate     /etc/ssl/pacs.example.crt;
    ssl_certificate_key /etc/ssl/pacs.example.key;

    # nginx refuses a body over 1 MB unless told otherwise; pacs-rs has its own bound
    client_max_body_size 0;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_request_buffering off;
        proxy_buffering off;
        proxy_read_timeout 660s;
    }
}
```

What was seen with it:

- **Addresses.** Through the proxy, a study's Retrieve URL is
  `https://pacs.example/dicom-web/studies/...`; asked directly, it is the server's own
  address. Without the two `X-Forwarded-*` headers a viewer is handed addresses it cannot
  reach.
- **Body size.** With nginx's defaults a 2.5 MB upload is answered 413 by nginx and never
  reaches pacs-rs. With `client_max_body_size 0` it does, and pacs-rs's own bound
  (`PACS_MAX_UPLOAD_BYTES`, 1 GiB) applies.

## What pacs-rs must be told

| Setting | When |
|---|---|
| `PACS_LISTEN=127.0.0.1:8080` | always: only the proxy should reach it. In Docker, publish the port on 127.0.0.1, or put both on one network and publish nothing |
| `PACS_AUTH=true` | before the proxy is opened to a network ([Login](login.md)) |
| `PACS_AUTH_COOKIE_SECURE=true` | the default; leave it. The cookie is then sent over HTTPS only, which the proxy provides |
| `PACS_BASE_URL=https://pacs.example/dicom-web` | when the proxy cannot set the `X-Forwarded-*` headers: the Retrieve URLs are then built from this |
| `PACS_MAX_REQUESTS_PER_CLIENT` | raise it, or set it to `0`: see below |
| `PACS_CORS_ORIGINS` | only when a viewer served from another address calls pacs-rs from the browser |

**Every visitor looks like the proxy.** pacs-rs sees the address that connected to it,
which is the proxy's. Two things follow:

- The bound on requests at once for one address (`PACS_MAX_REQUESTS_PER_CLIENT`, 64) is
  shared by everyone behind the proxy. Raise it towards `PACS_MAX_REQUESTS`, or set it to
  `0` and bound each visitor at the proxy instead
  ([Limits on requests](running.md#limits-on-requests)).
- The [audit trail](audit.md) records the proxy's address as where each request came
  from. Who made the request is still there, from the login. `X-Forwarded-For` is not read
  yet.

## DIMSE does not go through it

The DICOM network protocol is not HTTP: an HTTP proxy cannot carry it. Publish the DIMSE
port (`PACS_DIMSE_LISTEN`) on its own, to the machines that need it, and protect it with
[TLS and certificates](dimse.md#dimse-over-tls). A proxy that forwards TCP as it is
(nginx's `stream`, HAProxy in TCP mode) can sit in front; pacs-rs then sees its address for
every association.

## Not covered here

Serving pacs-rs under a path of another site (`https://example/pacs/`) is not described:
the cookie's path has a setting for it (`PACS_AUTH_COOKIE_PATH`), and the rest has not been
tried.
