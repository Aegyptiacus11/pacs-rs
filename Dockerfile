# pacs-server: build with the pinned toolchain, run on Debian slim as an unprivileged user.
FROM rust:1.99.1-slim-trixie AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p pacs-server

FROM debian:trixie-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --home-dir /data pacs \
    && mkdir -p /data && chown pacs /data
COPY --from=build /src/target/release/pacs-server /usr/local/bin/pacs-server
USER pacs
ENV PACS_LISTEN=0.0.0.0:8080 \
    PACS_INDEX=/data/index.sqlite \
    PACS_STORAGE=file:/data/files
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["pacs-server"]
