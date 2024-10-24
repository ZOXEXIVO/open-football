# BUILD FRONTEND
FROM node:22-alpine3.19 AS build-frontend

WORKDIR /app

COPY ./ui/package.json .

RUN npm install --legacy-peer-deps

COPY ./ui/ .

RUN npm run publish 

# BUILD BACKEND

FROM rust:1.82 as build-backend
WORKDIR /src

COPY ./ ./

# RUN TESTS

RUN cargo test -p core

# BUILD RELEASE

RUN cargo build --release

FROM rust:1.82-slim
WORKDIR /app

COPY --from=build-backend /src/target/release/football_simulator .
COPY --from=build-frontend /app/dist ./dist

ENTRYPOINT ["./football_simulator"]