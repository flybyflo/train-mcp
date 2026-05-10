FROM rust:1.93.1-bookworm AS builder

WORKDIR /app

# Native dependencies required by reqwest/native-tls.
RUN apt-get update \
    && apt-get install -y --no-install-recommends pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Cache dependencies first.
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY sql ./sql

RUN cargo build --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/train-mcp /usr/local/bin/train-mcp

ENV PORT=8000
EXPOSE 8000

CMD ["train-mcp"]
