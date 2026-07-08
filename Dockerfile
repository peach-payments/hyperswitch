# Base toolchain image shared by the recipe (planner) and build (builder)
# stages. cargo-chef lets dependency compilation live in its own cached layer
# that is only rebuilt when Cargo.toml/Cargo.lock change — combined with a
# buildx registry cache this skips the multi-hundred-crate dependency build on
# most pipelines.
FROM public.ecr.aws/docker/library/rust:trixie AS chef

# Use HTTPS apt sources — the build network blocks outbound HTTP (port 80).
RUN sed -i 's|http://|https://|g' /etc/apt/sources.list.d/debian.sources \
    && apt-get update \
    && apt-get install -y libpq-dev libssl-dev pkg-config protobuf-compiler \
    && cargo install cargo-chef --locked

WORKDIR /router

# Disable incremental compilation.
#
# Incremental compilation is useful as part of an edit-build-test-edit cycle,
# as it lets the compiler avoid recompiling code that hasn't changed. However,
# on CI, we're not making small edits; we're almost always building the entire
# project from scratch. Thus, incremental compilation on CI actually
# introduces *additional* overhead to support making future builds
# faster...but no future builds will ever occur in any given CI environment.
#
# See https://matklad.github.io/2021/09/04/fast-rust-builds.html#ci-workflow
# for details.
ENV CARGO_INCREMENTAL=0
# Allow more retries for network requests in cargo (downloading crates) and
# rustup (installing toolchains). This should help to reduce flaky CI failures
# from transient network timeouts or other issues.
ENV CARGO_NET_RETRY=10
ENV RUSTUP_MAX_RETRIES=10
# Don't emit giant backtraces in the CI logs.
ENV RUST_BACKTRACE="short"

# Compute the dependency recipe — only changes when Cargo.toml/Cargo.lock do.
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# Cook dependencies from the recipe (cached until deps change), then build the
# workspace. The cook args MUST match the final cargo build so the cached deps
# are reused.
FROM chef AS builder
ARG EXTRA_FEATURES=""
ARG VERSION_FEATURE_SET="v1"
COPY --from=planner /router/recipe.json recipe.json
RUN cargo chef cook \
    --release \
    --no-default-features \
    --features release \
    --features ${VERSION_FEATURE_SET} \
    --features redis-rs \
    ${EXTRA_FEATURES} \
    --recipe-path recipe.json

COPY . .
RUN cargo build \
    --release \
    --no-default-features \
    --features release \
    --features ${VERSION_FEATURE_SET} \
    --features redis-rs \
    ${EXTRA_FEATURES}



FROM public.ecr.aws/docker/library/debian:trixie

# Placing config and binary executable in different directories
ARG CONFIG_DIR=/local/config
ARG BIN_DIR=/local/bin

# Copy this required fields config file
COPY --from=builder /router/config/payment_required_fields_v2.toml ${CONFIG_DIR}/payment_required_fields_v2.toml

# RUN_ENV decides the corresponding config file to be used
ARG RUN_ENV=sandbox

# args for deciding the executable to export. three binaries:
# 1. BINARY=router - for main application
# 2. BINARY=scheduler, SCHEDULER_FLOW=consumer - part of process tracker
# 3. BINARY=scheduler, SCHEDULER_FLOW=producer - part of process tracker
ARG BINARY=router
ARG SCHEDULER_FLOW=consumer

# Use HTTPS apt sources (build network blocks outbound HTTP). Bring a CA bundle
# from the builder and point apt at it explicitly (Acquire::https::CAInfo) so it
# can verify TLS before ca-certificates is installed on this bare image.
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN printf 'Acquire::https::CAInfo "/etc/ssl/certs/ca-certificates.crt";\n' > /etc/apt/apt.conf.d/99-ca-info \
    && sed -i 's|http://|https://|g' /etc/apt/sources.list.d/debian.sources \
    && apt-get update \
    && apt-get install -y ca-certificates tzdata libpq-dev curl procps

EXPOSE 8080

ENV TZ=Etc/UTC \
    RUN_ENV=${RUN_ENV} \
    CONFIG_DIR=${CONFIG_DIR} \
    SCHEDULER_FLOW=${SCHEDULER_FLOW} \
    BINARY=${BINARY} \
    RUST_MIN_STACK=6291456

RUN mkdir -p ${BIN_DIR}

COPY --from=builder /router/target/release/${BINARY} ${BIN_DIR}/${BINARY}

# Create the 'app' user and group
RUN useradd --user-group --system --no-create-home --no-log-init app
USER app:app

WORKDIR ${BIN_DIR}

CMD ./${BINARY}
