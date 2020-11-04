FROM rust:1.47 as build
WORKDIR /src

COPY ./ ./

RUN cargo test -p core

RUN cargo build --release

FROM rust:1.47-slim
WORKDIR /app
COPY --from=build /src/target/release .

ENTRYPOINT ["./football_simulator"]