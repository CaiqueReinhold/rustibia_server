FROM rust:1.95-slim AS builder
WORKDIR /app

# Build context is the workspace root, so both manifests and the shared lockfile are
# visible. The site's manifest is required for the workspace to resolve at all.
COPY Cargo.toml Cargo.lock ./
COPY crates/server/Cargo.toml crates/server/Cargo.toml
COPY crates/site/Cargo.toml   crates/site/Cargo.toml

# The site's sources are needed even though only the server is built: sqlx::migrate!()
# and askama read migrations/ and templates/ at compile time, so a stub will not do.
COPY crates/site/ crates/site/
COPY crates/server/src/ crates/server/src/

RUN cargo build --release -p rustibia-server

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/server .
COPY crates/server/assets/ assets/
EXPOSE 5555
CMD ["./server"]
