# Define Rust version
ARG RUST_VERSION=1.93

# BUILD BACKEND

FROM rust:${RUST_VERSION} as build-backend
WORKDIR /src

COPY ./ ./

# RUN TESTS

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/home/root/app/target \
    cargo test -p core

# BUILD RELEASE

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/home/root/app/target \
    cargo build --release

FROM rust:${RUST_VERSION}-slim
WORKDIR /app

COPY --from=build-backend /src/target/release/open_football .

ENTRYPOINT ["./open_football"]