# pacs-server: a static musl binary on an empty image, with only the CA bundle that S3 over
# HTTPS needs. Runs as an unprivileged user; index and files in /data.
FROM rust:1.99.1-alpine3.24 AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
COPY . .
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
ENTRYPOINT ["/pacs-server"]
