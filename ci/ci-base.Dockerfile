# CI toolchain image for the static-analysis job.
#
# Bundles Rust + clippy + a nightly toolchain with rustfmt + `just` + the apt
# build deps, and PRE-WARMS the cargo registry and compiled dependencies (for
# both the v1 and v2 feature sets) so the checks job only recompiles the
# workspace crates instead of the whole dependency tree.
#
# Built and pushed to $CI_REGISTRY_IMAGE/ci-base by the build-ci-image job.
# Rebuild it when dependencies (Cargo.lock / Cargo.toml) or this file change.
FROM public.ecr.aws/docker/library/rust:trixie

ENV CARGO_INCREMENTAL=0 \
    CARGO_NET_RETRY=10 \
    RUSTUP_MAX_RETRIES=10 \
    RUST_BACKTRACE=short \
    CARGO_TARGET_DIR=/warm/target

# HTTPS apt sources — the build network blocks outbound HTTP (port 80).
RUN sed -i 's|http://|https://|g' /etc/apt/sources.list.d/debian.sources \
    && apt-get update \
    && apt-get install -y libpq-dev libssl-dev pkg-config protobuf-compiler jq curl \
    && rm -rf /var/lib/apt/lists/*

# clippy + a nightly toolchain with rustfmt (the repo formats with nightly), + just.
RUN rustup component add clippy \
    && rustup toolchain install nightly --component rustfmt --profile minimal \
    && curl --proto '=https' --tlsv1.2 -sSf https://just.systems/install.sh | bash -s -- --to /usr/local/bin

# Pre-warm the cargo registry + compiled dependencies for both feature sets,
# using the repo's own recipes so the features match CI exactly. The lint result
# is intentionally ignored (|| true) — the goal is a populated target dir and
# cargo cache, not a gate. CARGO_TARGET_DIR / CARGO_HOME are baked into the image
# so the checks job reuses these artifacts.
WORKDIR /warm
COPY . .
RUN just clippy "redis-rs" -- -D warnings || true
RUN just clippy_v2 "redis-rs" || true
