# syntax=docker/dockerfile:1

FROM rust:1.96-bookworm AS chef
WORKDIR /app
COPY rust-toolchain.toml ./
RUN cargo install cargo-chef --locked

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS backend
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --locked --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --locked --release

FROM debian:bookworm-slim AS runtime
WORKDIR /app
COPY --from=backend /app/target/release/home-link /usr/local/bin/home-link
RUN mkdir /data && chown 10001:10001 /data
ENV PORT=3000 DATABASE_PATH=/data/home-link.db
EXPOSE 3000
VOLUME /data
USER 10001:10001
ENTRYPOINT ["home-link"]
