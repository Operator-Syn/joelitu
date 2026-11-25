# syntax=docker/dockerfile:1.7

# Builder: full Rust toolchain, throwaway.
FROM rust:1.85-bookworm AS builder

WORKDIR /app

# Eight parallel rustc processes. The host has sixteen cores, but each rustc can
# spike past a gigabyte, so half the cores keeps a release build from pushing the
# box into swap while other services run.
ENV CARGO_BUILD_JOBS=8

# Copy manifests first so the dependency graph is resolved in a layer that only
# changes when the manifests change.
COPY Cargo.toml Cargo.lock ./
COPY rust-toolchain.toml ./

# The cache mounts are the real build cache: the registry keeps downloaded crates
# and `target/` keeps compiled dependencies. Both live outside the image layer, so
# the binary has to be copied out inside this same RUN — a `COPY` in a later stage
# cannot see a mount.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    mkdir -p src && \
    printf 'fn main() {}\n' > src/main.rs && \
    cargo build --release && \
    rm -rf src

# Real sources last, so editing them invalidates only the final crate compile.
COPY src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && \
    cp target/release/joelitu /usr/local/bin/joelitu

# Runtime: glibc, because the GNU target needs it. musl would allow `scratch`, but
# the TLS stack needs extra build tooling for a few megabytes nobody is short of.
FROM debian:bookworm-slim AS runtime

# rustls bundles its own roots, so this is not strictly required; it is installed
# so that any future tooling that shells out to curl behaves.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 10001 --create-home --shell /usr/sbin/nologin joelitu

COPY --from=builder /usr/local/bin/joelitu /usr/local/bin/joelitu

USER joelitu
ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/joelitu"]
