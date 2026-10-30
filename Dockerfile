# pacs-server: a static musl binary on an empty image, with only the CA bundle that S3 over
# HTTPS needs. Runs as an unprivileged user; index and files in /data.

# The web interface (ui/), built here and embedded in the binary by build.rs.
FROM node:24.22.0-alpine3.24 AS ui
WORKDIR /ui
COPY ui/package.json ui/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY ui/ ./
RUN npm run build

FROM rust:1.99.1-alpine3.24 AS build
# g++ and cmake build CharLS (JPEG-LS), which is linked in statically.
RUN apk add --no-cache musl-dev g++ cmake make
WORKDIR /src
COPY . .
COPY --from=ui /ui/dist ui/dist
RUN cargo build --release --locked -p pacs-server \
    && mkdir -p /out/data

FROM scratch
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
COPY --from=build /src/target/release/pacs-server /pacs-server
COPY --from=build --chown=10001:10001 /out/data /data
USER 10001:10001
ENV PACS_LISTEN=0.0.0.0:8080 \
    PACS_INDEX=/data/index.sqlite \
    PACS_STORAGE=file:/data/files \
    SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=6s --start-period=5s CMD ["/pacs-server", "--healthcheck"]
ENTRYPOINT ["/pacs-server"]
