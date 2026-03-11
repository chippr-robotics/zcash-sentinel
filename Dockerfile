# Multi-stage build for zcash-watchman
# zingolib v3.0.0 requires Rust edition 2024 (channel 1.90+)
FROM rust:1.90-bookworm AS builder

WORKDIR /build

# Install protobuf compiler (required by zingolib gRPC dependencies)
RUN apt-get update && apt-get install -y protobuf-compiler && rm -rf /var/lib/apt/lists/*

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock* ./

# Create dummy src to cache dependency builds
RUN mkdir src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release 2>/dev/null || true

# Copy actual source and rebuild
COPY src/ src/
RUN touch src/main.rs && cargo build --release

# Runtime image
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/zcash-watchman /usr/local/bin/

# Create data directory
RUN mkdir -p /var/lib/zcash-watchman

EXPOSE 9100 9101

ENTRYPOINT ["zcash-watchman"]
CMD ["--config", "/etc/zcash-watchman/config.toml"]
