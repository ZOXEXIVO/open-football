# Multi-platform release builder
ARG RUST_VERSION=1.96

# ── Windows x86_64 ────────────────────────────────────────────────────

FROM rust:${RUST_VERSION} AS build-windows
WORKDIR /src
COPY ./ ./

RUN sed -i 's|http://deb.debian.org/debian|https://mirror.yandex.ru/debian|g' \
        /etc/apt/sources.list.d/debian.sources \
 && apt-get -o Acquire::Retries=5 update \
 && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 zip \
 && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-pc-windows-gnu
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target/x86_64-pc-windows-gnu \
    cargo build --release --target x86_64-pc-windows-gnu && \
    mkdir -p /dist && \
    cp target/x86_64-pc-windows-gnu/release/open_football.exe /dist/ && \
    cd /dist && zip open-football-windows-x86_64.zip open_football.exe

# ── Linux x86_64 ──────────────────────────────────────────────────────

FROM rust:${RUST_VERSION} AS build-linux
WORKDIR /src
COPY ./ ./

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target/release \
    cargo build --release && \
    mkdir -p /dist && \
    cp target/release/open_football /dist/ && \
    cd /dist && tar czf open-football-linux-x86_64.tar.gz open_football

# ── Publish GitHub Release ────────────────────────────────────────────

FROM alpine:latest AS publish

ARG DRONE_TAG
ARG DRONE_REPO

RUN sed -i 's|https://dl-cdn.alpinelinux.org/alpine|https://mirror.yandex.ru/mirrors/alpine|g' \
        /etc/apk/repositories \
 && apk add --no-cache curl jq

WORKDIR /release
COPY --from=build-windows /dist/open-football-windows-x86_64.zip .
COPY --from=build-linux /dist/open-football-linux-x86_64.tar.gz .
COPY build/publish-release.sh /usr/local/bin/publish-release.sh

RUN --mount=type=secret,id=github_token sh /usr/local/bin/publish-release.sh
