# CI toolchain image for the static-analysis job.
#
# Bundles Rust + clippy + a nightly toolchain with rustfmt + `just` + the apt
# build deps, and PRE-WARMS the cargo registry and compiled dependencies (for
# both the v1 and v2 feature sets) so the checks job only recompiles the
# workspace crates instead of the whole dependency tree.
#
# Built and pushed to $CI_REGISTRY_IMAGE/ci-base by the build-ci-image job.
# Rebuild it when dependencies (Cargo.lock / Cargo.toml) or this file change.
#
# Pin the Rust version. The `rust:trixie` tag is rolling, so its default toolchain
# silently advances (it reached 1.97, which promotes several clippy lints to
# warn-by-default and breaks the `-D warnings` static-analysis gate on unchanged
# code). Pin to a known-good version and bump deliberately after confirming the
# workspace still passes `cargo clippy … -- -D warnings`. Keep >= the MSRV in
# Cargo.toml (package.rust-version).
FROM public.ecr.aws/docker/library/rust:1.93-trixie

ENV CARGO_INCREMENTAL=0 \
    CARGO_NET_RETRY=10 \
    RUSTUP_MAX_RETRIES=10 \
    RUST_BACKTRACE=short \
    CARGO_TARGET_DIR=/warm/target

# HTTPS apt sources — the build network blocks outbound HTTP (port 80).
# NOTE: when the migration-check job is enabled, also add `postgresql-client`
# here (psql) and `cargo install diesel_cli --no-default-features --features
# postgres --locked` below.
RUN sed -i 's|http://|https://|g' /etc/apt/sources.list.d/debian.sources \
    && apt-get update \
    && apt-get install -y libpq-dev libssl-dev pkg-config protobuf-compiler jq curl \
    && rm -rf /var/lib/apt/lists/*

# clippy + a nightly toolchain with rustfmt (the repo formats with nightly), + just.
RUN rustup component add clippy \
    && rustup toolchain install nightly --component rustfmt --profile minimal \
    && cargo install just --locked

# Pre-warm the cargo registry + compiled dependencies for the shipped feature
# set (release + v1 + redis-rs), matching the static-analysis clippy invocation
# so its artifacts are reused. The lint result is intentionally ignored
# (|| true) — the goal is a populated target dir and cargo cache, not a gate.
# CARGO_TARGET_DIR / CARGO_HOME are baked into the image so the checks job reuses
# these artifacts.
WORKDIR /warm
COPY . .
RUN cargo clippy --no-default-features --features release --features v1 --features redis-rs -- -D warnings || true