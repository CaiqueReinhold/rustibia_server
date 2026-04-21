FROM rust:1.95-slim AS builder
WORKDIR /app

# Cache dependencies before copying source
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release
RUN rm -f target/release/deps/server* src/main.rs

# Build real binary (migrations must be present for sqlx::migrate!())
COPY migrations/ migrations/
COPY src/ src/
RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/server .
COPY assets/ assets/
EXPOSE 5555
CMD ["./server"]
