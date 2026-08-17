# Define Rust version
ARG RUST_VERSION=1.97

# BUILD BACKEND

FROM rust:${RUST_VERSION} AS build-backend
WORKDIR /src

# `src/web/build.rs` compiles the Bevy match viewer (src/match) to WebAssembly
# and embeds it. Without this target the build still succeeds, but the shipped
# image serves match pages with no replay.
RUN rustup target add wasm32-unknown-unknown

COPY ./ ./

# RUN TESTS

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/home/root/app/target \
    cargo test -p core

# BUILD RELEASE

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/home/root/app/target \
    --mount=type=cache,target=/src/src/match/target \
    cargo build --release

FROM rust:${RUST_VERSION}-slim
WORKDIR /app

COPY --from=build-backend /src/target/release/open_football .

ENTRYPOINT ["./open_football"]
