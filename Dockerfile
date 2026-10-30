FROM node:26.12.0-trixie-slim@sha256:e556a144463cc8dc53afe8de6d2fa0db5c132fd39ffa232c17641bd40df2bf11 AS web
WORKDIR /usr/local/src/spoticlean/web
COPY web/package.json web/package-lock.json ./
RUN npm ci
COPY web/ ./
RUN npm run build

FROM rust:1.99.0-slim-trixie@sha256:2752b332db73fdbb7dc576f06c82ed1f312005784ef913d7e04a28f5f55dc581 AS chef
WORKDIR /usr/local/src/spoticlean
RUN cargo install cargo-chef --version 0.1.78 --locked

FROM chef AS planner
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY rust-toolchain.toml ./
COPY --from=planner /usr/local/src/spoticlean/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM gcr.io/distroless/cc-debian13:nonroot@sha256:3ee00291cf386ec52a552e6a858d555debd74776eb4bfa53fa3df2cd6677515e
COPY --from=builder /usr/local/src/spoticlean/target/release/spoticlean /usr/local/bin/spoticlean
COPY --from=web /usr/local/src/spoticlean/web/dist /usr/share/spoticlean/web
ENV STATIC_DIR=/usr/share/spoticlean/web
USER 65532:65532
EXPOSE 8080
HEALTHCHECK CMD ["/usr/local/bin/spoticlean", "healthcheck"]
ENTRYPOINT ["/usr/local/bin/spoticlean"]
CMD ["serve"]
