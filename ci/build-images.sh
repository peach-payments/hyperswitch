#!/usr/bin/env bash
#
# Build the four deployable Hyperswitch images from the root Dockerfile and push
# them to the GitLab Container Registry, tagged with the commit SHA.
#
# All four images share an identical builder stage, so building them against a
# single Docker daemon compiles the Rust workspace once and reuses the cached
# layer for the remaining three. The per-env release jobs (ci/release-ecr.sh)
# then promote these images to each CDE environment's ECR.
#
# Expects the caller to have already logged docker in to $CI_REGISTRY.
#
# Driven by:
#   CI_REGISTRY_IMAGE     GitLab registry base for this project
#   CI_COMMIT_SHORT_SHA   short commit sha (the image tag)
#   VERSION_FEATURE_SET   cargo feature set (v1 / v2), defaults to v1

set -euo pipefail

VERSION_FEATURE_SET="${VERSION_FEATURE_SET:-v1}"
: "${CI_REGISTRY_IMAGE:?}"
: "${CI_COMMIT_SHORT_SHA:?}"

# name <- "BINARY[,SCHEDULER_FLOW]". router and drainer are distinct binaries;
# producer and consumer are the same `scheduler` binary switched by SCHEDULER_FLOW.
build_and_push() {
  local name="$1" binary="$2" scheduler_flow="${3:-}"
  local image="${CI_REGISTRY_IMAGE}/hyperswitch-${name}:${CI_COMMIT_SHORT_SHA}"

  echo "==> building ${image} (BINARY=${binary}${scheduler_flow:+, SCHEDULER_FLOW=${scheduler_flow}})"
  local args=(
    --build-arg "BINARY=${binary}"
    --build-arg "VERSION_FEATURE_SET=${VERSION_FEATURE_SET}"
  )
  if [ -n "$scheduler_flow" ]; then
    args+=(--build-arg "SCHEDULER_FLOW=${scheduler_flow}")
  fi
  docker build "${args[@]}" -t "$image" .

  echo "==> pushing ${image}"
  docker push "$image"
}

build_and_push router   router
build_and_push producer scheduler producer
build_and_push consumer scheduler consumer
build_and_push drainer  drainer

echo "==> done"
