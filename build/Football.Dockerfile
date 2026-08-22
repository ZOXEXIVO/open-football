# Define Rust version
ARG RUST_VERSION=1.98

# BUILD BACKEND

FROM rust:${RUST_VERSION} AS build-backend
WORKDIR /src

# `src/web/build.rs` compiles the Bevy match viewer (src/match) to WebAssembly
# and embeds it. Without this target the build still succeeds, but the shipped
# image serves match pages with no replay.
RUN rustup target add wasm32-unknown-unknown

COPY ./ ./

# RUN TESTS

# The cache mount has to name the directory cargo actually writes to. It read
# `/home/root/app/target` — a path nothing in this image ever creates — so
# every build recompiled all 200-odd dependencies, `core`'s 400k lines and a
# fat-LTO link from nothing. `WORKDIR` is `/src`, so the target dir is
# `/src/target`.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo test -p core

# BUILD RELEASE

# A cache mount is not part of the resulting layer, so the binary has to be
# lifted out of it here — `COPY --from` cannot reach inside one.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    --mount=type=cache,target=/src/src/match/target \
    cargo build --release \
    && cp /src/target/release/open_football /open_football

FROM rust:${RUST_VERSION}-slim
WORKDIR /app

COPY --from=build-backend /open_football .

ENTRYPOINT ["./open_football"]
