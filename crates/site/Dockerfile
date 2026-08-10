FROM rust:1.95-slim AS builder
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates/server/Cargo.toml crates/server/Cargo.toml
COPY crates/site/Cargo.toml   crates/site/Cargo.toml

# migrations/ and templates/ are read at compile time by sqlx::migrate!() and askama.
COPY crates/site/ crates/site/

# The server's sources are needed even though only the site is built. Its manifest
# declares `[[bin]] path = "src/main.rs"`, and cargo validates that every declared
# target exists while resolving the workspace — a missing file fails the build before
# it starts. Do not delete this line to "optimise" the image.
COPY crates/server/src/ crates/server/src/

RUN cargo build --release -p rustibia-site

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/rustibia-site .
COPY crates/site/config.yaml ./config.yaml
COPY crates/site/static/ static/
EXPOSE 8080
CMD ["./rustibia-site"]
