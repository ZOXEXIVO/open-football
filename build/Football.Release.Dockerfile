# Multi-platform release builder
ARG RUST_VERSION=1.94

# ── Windows x86_64 ────────────────────────────────────────────────────

FROM rust:${RUST_VERSION} AS build-windows
WORKDIR /src
COPY ./ ./

RUN apt-get update && apt-get install -y gcc-mingw-w64-x86-64 zip
RUN rustup target add x86_64-pc-windows-gnu
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --target x86_64-pc-windows-gnu

RUN mkdir -p /dist && \
    cp target/x86_64-pc-windows-gnu/release/open_football.exe /dist/ && \
    cd /dist && zip open-football-windows-x86_64.zip open_football.exe

# ── Linux x86_64 ──────────────────────────────────────────────────────

FROM rust:${RUST_VERSION} AS build-linux
WORKDIR /src
COPY ./ ./

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release

RUN mkdir -p /dist && \
    cp target/release/open_football /dist/ && \
    cd /dist && tar czf open-football-linux-x86_64.tar.gz open_football

# ── macOS x86_64 (cross-compile) ─────────────────────────────────────

FROM rust:${RUST_VERSION} AS build-macos-x86_64
WORKDIR /src
COPY ./ ./

RUN apt-get update && apt-get install -y clang cmake libssl-dev lzma-dev libxml2-dev zip
RUN rustup target add x86_64-apple-darwin

RUN git clone --depth 1 https://github.com/nicktrav/osxcross /opt/osxcross-src && \
    cd /opt/osxcross-src && \
    wget -nc https://github.com/nicktrav/osxcross/releases/download/v1.0/MacOSX14.0.sdk.tar.xz -O tarballs/MacOSX14.0.sdk.tar.xz && \
    UNATTENDED=1 ./build.sh

ENV PATH="/opt/osxcross-src/target/bin:$PATH" \
    CC_x86_64_apple_darwin=x86_64-apple-darwin23-clang \
    CXX_x86_64_apple_darwin=x86_64-apple-darwin23-clang++ \
    CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER=x86_64-apple-darwin23-clang

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --target x86_64-apple-darwin

RUN mkdir -p /dist && \
    cp target/x86_64-apple-darwin/release/open_football /dist/ && \
    cd /dist && tar czf open-football-macos-x86_64.tar.gz open_football

# ── macOS aarch64 (cross-compile) ────────────────────────────────────

FROM rust:${RUST_VERSION} AS build-macos-aarch64
WORKDIR /src
COPY ./ ./

RUN apt-get update && apt-get install -y clang cmake libssl-dev lzma-dev libxml2-dev zip
RUN rustup target add aarch64-apple-darwin

RUN git clone --depth 1 https://github.com/nicktrav/osxcross /opt/osxcross-src && \
    cd /opt/osxcross-src && \
    wget -nc https://github.com/nicktrav/osxcross/releases/download/v1.0/MacOSX14.0.sdk.tar.xz -O tarballs/MacOSX14.0.sdk.tar.xz && \
    UNATTENDED=1 ./build.sh

ENV PATH="/opt/osxcross-src/target/bin:$PATH" \
    CC_aarch64_apple_darwin=aarch64-apple-darwin23-clang \
    CXX_aarch64_apple_darwin=aarch64-apple-darwin23-clang++ \
    CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=aarch64-apple-darwin23-clang

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --target aarch64-apple-darwin

RUN mkdir -p /dist && \
    cp target/aarch64-apple-darwin/release/open_football /dist/ && \
    cd /dist && tar czf open-football-macos-aarch64.tar.gz open_football

# ── Publish GitHub Release ────────────────────────────────────────────

FROM alpine:latest AS publish

ARG GITHUB_TOKEN
ARG DRONE_TAG
ARG DRONE_REPO

RUN apk add --no-cache curl jq

WORKDIR /release
COPY --from=build-windows /dist/open-football-windows-x86_64.zip .
COPY --from=build-linux /dist/open-football-linux-x86_64.tar.gz .
COPY --from=build-macos-x86_64 /dist/open-football-macos-x86_64.tar.gz .
COPY --from=build-macos-aarch64 /dist/open-football-macos-aarch64.tar.gz .

RUN VERSION="${DRONE_TAG#release-}" && \
    RELEASE_ID=$(curl -sf -X POST \
      -H "Authorization: token ${GITHUB_TOKEN}" \
      -H "Content-Type: application/json" \
      -d "{\"tag_name\":\"${DRONE_TAG}\",\"name\":\"OpenFootball Release v${VERSION}\"}" \
      "https://api.github.com/repos/${DRONE_REPO}/releases" \
      | jq -r '.id') && \
    for FILE in /release/*; do \
      curl -sf -X POST \
        -H "Authorization: token ${GITHUB_TOKEN}" \
        -H "Content-Type: application/octet-stream" \
        --data-binary "@${FILE}" \
        "https://uploads.github.com/repos/${DRONE_REPO}/releases/${RELEASE_ID}/assets?name=$(basename ${FILE})"; \
    done
