# syntax=docker/dockerfile:1

# Define Rust version
ARG RUST_VERSION=1.98

# ── Source ────────────────────────────────────────────────────────────
#
# The toolchain and the source copy, shared by every stage below. Split out
# so `COPY ./ ./` happens ONCE: `test` and `build-backend` run concurrently
# and would otherwise each pull the 53 MB context and each pay the layer.

FROM rust:${RUST_VERSION} AS source
WORKDIR /src

# `src/web/build.rs` compiles the Bevy match viewer (src/match) to WebAssembly
# and embeds it. Without this target the build still succeeds, but the shipped
# image serves match pages with no replay.
RUN rustup target add wasm32-unknown-unknown

COPY ./ ./

# ── RUN TESTS ─────────────────────────────────────────────────────────
#
# Its own stage rather than a second `RUN` in front of the build, because
# BuildKit runs independent stages CONCURRENTLY and these two share no work:
# `cargo test` compiles the dev profile, `cargo build --release` the release
# one, so nothing the first produces is an input to the second. Serialised,
# the suite cost 231s in front of a 288s build; alongside it, it costs
# nothing — the release build is the longer of the two and stays the critical
# path.
#
# Cache ids are DISTINCT from the release stage's for the same reason
# `Football.Release.Dockerfile` gives: cargo's package lock lives at
# $CARGO_HOME/.package-cache, outside the mounted registry/ dir, so a shared
# registry mount gives two concurrent stages no mutual exclusion and they
# race unpacking the same crate (".cargo-ok: File exists"). The target dirs
# are separate for the same reason — plus cargo takes a lock on the target
# directory, which would serialise the two stages right back again.
FROM source AS test
RUN --mount=type=cache,id=cargo-registry-test,target=/usr/local/cargo/registry \
    --mount=type=cache,id=target-test,target=/src/target \
    cargo test -p core --locked \
    && touch /tests-passed

# ── BUILD RELEASE ─────────────────────────────────────────────────────

# A cache mount is not part of the resulting layer, so the binary has to be
# lifted out of it here — `COPY --from` cannot reach inside one.
#
# The cache mount has to name the directory cargo actually writes to. It read
# `/home/root/app/target` — a path nothing in this image ever creates — so
# every build recompiled all 200-odd dependencies, `core`'s 400k lines and a
# fat-LTO link from nothing. `WORKDIR` is `/src`, so the target dir is
# `/src/target`.
#
# The first three keep their DEFAULT cache ids — an id is the cache's
# identity, and it defaults to the target path, so naming them the way the
# test stage's are named would point them at empty directories and orphan the
# warm registry and target dir this build already has. Only the test stage
# needed new ids, and only because it now runs at the same time as this one.
#
# The last mount is the viewer's STAGED output, and it is what makes
# `MatchViewer::stage`'s fast path reachable in CI at all. That function
# fingerprints the compiled wasm and skips wasm-bindgen and a level-9 gzip
# over ~30 MB of Bevy when the staged files already match — but it decides
# "already staged" by looking in `src/web/assets/static/viewer/`, which is
# gitignored (it is generated), so it is EMPTY in every build context. The
# check therefore failed every time and the pipeline it guards ran every
# time. Mounted, the staged bytes outlive the build context and the skip
# works. `rust-embed` reads that directory while `web` compiles, which is
# inside this same RUN, so the mount is live when it matters.
#
# One consequence to know about: a nested wasm build that FAILS now leaves the
# previous viewer staged, so the image ships the last good one instead of
# none. Either way the only announcement is a `cargo:warning` from the build
# script — but the site keeps its replay.
FROM source AS build-backend
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    --mount=type=cache,target=/src/src/match/target \
    --mount=type=cache,id=match-viewer-assets,target=/src/src/web/assets/static/viewer \
    cargo build --release --locked \
    && cp /src/target/release/open_football /open_football

FROM rust:${RUST_VERSION}-slim
WORKDIR /app

COPY --from=build-backend /open_football .

# Nothing reads this file. It is here to put the `test` stage in the final
# image's dependency graph: BuildKit only builds a stage something pulls
# from, and without this edge the suite would never run — a green pipeline
# that tested nothing. Last, so the marker's mtime does not invalidate the
# binary layer above it.
COPY --from=test /tests-passed /tests-passed

ENTRYPOINT ["./open_football"]
