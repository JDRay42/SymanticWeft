# Stage 1: Build
# Uses the official Rust image to compile the node binary.
FROM rust:1.86-slim AS builder

WORKDIR /build

# Install build dependencies for rusqlite (bundled SQLite requires cc).
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifests first so dependency layers are cached.
COPY Cargo.toml Cargo.lock ./
COPY packages/core/Cargo.toml        packages/core/Cargo.toml
COPY packages/cli/Cargo.toml         packages/cli/Cargo.toml
COPY packages/agent-core/Cargo.toml  packages/agent-core/Cargo.toml
COPY packages/wasm/Cargo.toml        packages/wasm/Cargo.toml
COPY packages/node-api/Cargo.toml    packages/node-api/Cargo.toml
COPY packages/node/Cargo.toml        packages/node/Cargo.toml
COPY packages/conformance/Cargo.toml packages/conformance/Cargo.toml

# Stub out all library/binary sources so Cargo can resolve and cache deps.
RUN mkdir -p \
    packages/core/src \
    packages/cli/src \
    packages/agent-core/src \
    packages/wasm/src \
    packages/node-api/src \
    packages/node/src \
    packages/conformance/src \
    packages/conformance/tests \
  && echo "pub fn main() {}" > packages/cli/src/main.rs \
  && echo "pub fn main() {}" > packages/node/src/main.rs \
  && echo "fn main() {}" > packages/conformance/src/main.rs \
  && for p in core agent-core wasm node-api; do \
       echo "" > packages/$p/src/lib.rs; \
     done \
  && touch packages/node/src/lib.rs \
  && touch packages/conformance/tests/placeholder.rs

RUN cargo build --release -p semanticweft-node 2>&1 | tail -5

# Now copy real sources and rebuild only changed crates.
COPY packages/ packages/
RUN touch packages/node/src/main.rs \
  && cargo build --release -p semanticweft-node

# Stage 2: Runtime
# Minimal Debian image — no Rust toolchain, no build tools.
FROM debian:bookworm-slim

# CA certificates are needed for outbound HTTPS calls to federation peers.
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Non-root user for the process.
RUN useradd --system --create-home --uid 1001 sweft

COPY --from=builder /build/target/release/sweft-node /usr/local/bin/sweft-node

# Default data directory; override SWEFT_DB to place the file elsewhere.
RUN mkdir -p /data && chown sweft:sweft /data

USER sweft
WORKDIR /data

# Node API port (matches SWEFT_BIND default of 0.0.0.0:3000).
EXPOSE 3000

# Environment variable defaults — all can be overridden at runtime.
ENV SWEFT_BIND=0.0.0.0:3000
ENV SWEFT_DB=/data/node.db
ENV RUST_LOG=semanticweft_node=info

ENTRYPOINT ["/usr/local/bin/sweft-node"]
