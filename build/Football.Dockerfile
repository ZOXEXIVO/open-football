# Define Rust version
ARG RUST_VERSION=1.91

# BUILD FRONTEND
FROM node:22-alpine3.19 AS build-frontend

WORKDIR /app

COPY ./ui/package.json .

RUN npm install --force

COPY ./ui/ .

RUN npm run publish

# BUILD BACKEND

FROM rust:${RUST_VERSION} as build-backend
WORKDIR /src

COPY ./ ./

# RUN TESTS

RUN cargo test -p core

# BUILD RELEASE

RUN cargo build --release

FROM rust:${RUST_VERSION}-slim
WORKDIR /app

COPY --from=build-backend /src/target/release/open_football .
COPY --from=build-frontend /app/dist ./dist

ENTRYPOINT ["./open_football"]