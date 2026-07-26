# Multi-platform release builder
#
# All four build stages run concurrently, so each one gets its OWN cargo
# registry cache id. Cargo's package lock lives at $CARGO_HOME/.package-cache,
# outside the mounted registry/ dir — a shared cache mount therefore gives the
# stages no mutual exclusion and two of them racing to unpack the same crate
# fail with ".cargo-ok: File exists". Distinct ids cost a little disk and some
# repeat downloads; `sharing=locked` would be the alternative but it holds the
# lock for the whole RUN, serialising every platform build end to end.
ARG RUST_VERSION=1.97
# osxcross + macOS SDK + Rust, prebuilt. Only the osxcross half is used: the
# image's own Rust lags well behind (latest shipped 1.89, and `sysinfo` needs
# 1.95+), so the mac stages install RUST_VERSION over the top of it.
ARG DARWIN_BUILDER_VERSION=latest

# ── Windows x86_64 ────────────────────────────────────────────────────

FROM rust:${RUST_VERSION} AS build-windows
WORKDIR /src

# Toolchain before the source copy — otherwise every commit re-runs the ~45s
# mingw install.
RUN sed -i 's|http://deb.debian.org/debian|https://mirror.yandex.ru/debian|g' \
    /etc/apt/sources.list.d/debian.sources \
    && apt-get -o Acquire::Retries=5 update \
    && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-pc-windows-gnu

COPY ./ ./

RUN --mount=type=cache,id=cargo-registry-windows,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target/x86_64-pc-windows-gnu \
    cargo build --release --target x86_64-pc-windows-gnu && \
    mkdir -p /dist && \
    cp target/x86_64-pc-windows-gnu/release/open_football.exe /dist/

# ── Linux x86_64 ──────────────────────────────────────────────────────

FROM rust:${RUST_VERSION} AS build-linux
WORKDIR /src
COPY ./ ./

RUN --mount=type=cache,id=cargo-registry-linux,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target/release \
    cargo build --release && \
    mkdir -p /dist && \
    cp target/release/open_football /dist/

# ── macOS x86_64 (Intel) ──────────────────────────────────────────────

FROM joseluisq/rust-linux-darwin-builder:${DARWIN_BUILDER_VERSION} AS build-mac-intel
ARG RUST_VERSION
WORKDIR /src

# Toolchain first, so a source-only change does not re-run it. `rustup target`
# is per-toolchain, hence after the default switch. The image's osxcross linker
# wiring lives in /root/.cargo/config and is unaffected by the Rust bump.
RUN rustup toolchain install ${RUST_VERSION} --profile minimal \
    && rustup default ${RUST_VERSION} \
    && rustup target add x86_64-apple-darwin

COPY ./ ./

# `sysinfo` (IOKit) and `chrono`→`iana-time-zone` (CoreFoundation) link real
# Apple frameworks, so these builds go through the image's osxcross clang.
# CARGO_HOME is /root/.cargo in this image, not /usr/local/cargo.
ENV CC=o64-clang CXX=o64-clang++
RUN --mount=type=cache,id=cargo-registry-mac-intel,target=/root/.cargo/registry \
    --mount=type=cache,target=/src/target/x86_64-apple-darwin \
    cargo build --release --target x86_64-apple-darwin && \
    mkdir -p /dist && \
    cp target/x86_64-apple-darwin/release/open_football /dist/

# ── macOS aarch64 (M series) ──────────────────────────────────────────

FROM joseluisq/rust-linux-darwin-builder:${DARWIN_BUILDER_VERSION} AS build-mac-m-series
ARG RUST_VERSION
WORKDIR /src

RUN rustup toolchain install ${RUST_VERSION} --profile minimal \
    && rustup default ${RUST_VERSION} \
    && rustup target add aarch64-apple-darwin

COPY ./ ./

ENV CC=oa64-clang CXX=oa64-clang++
RUN --mount=type=cache,id=cargo-registry-mac-m-series,target=/root/.cargo/registry \
    --mount=type=cache,target=/src/target/aarch64-apple-darwin \
    cargo build --release --target aarch64-apple-darwin && \
    mkdir -p /dist && \
    cp target/aarch64-apple-darwin/release/open_football /dist/

# ── Publish GitHub Release ────────────────────────────────────────────

FROM alpine:latest AS publish

ARG DRONE_TAG
ARG DRONE_REPO

RUN sed -i 's|https://dl-cdn.alpinelinux.org/alpine|https://mirror.yandex.ru/mirrors/alpine|g' \
    /etc/apk/repositories \
    && apk add --no-cache curl jq zip tar

# Raw binaries land here; archiving happens in this stage so that a new tag
# only re-runs the packaging step and never invalidates the build caches.
WORKDIR /artifacts
COPY --from=build-windows      /dist/open_football.exe ./windows/
COPY --from=build-linux        /dist/open_football     ./linux/
COPY --from=build-mac-intel    /dist/open_football     ./mac_intel/
COPY --from=build-mac-m-series /dist/open_football     ./mac_m_series/

COPY build/publish-release.sh /usr/local/bin/publish-release.sh
COPY build/package-release.sh /usr/local/bin/package-release.sh

RUN sh /usr/local/bin/package-release.sh
RUN --mount=type=secret,id=github_token sh /usr/local/bin/publish-release.sh
