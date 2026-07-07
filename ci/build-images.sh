#!/usr/bin/env bash
#
# Build the four deployable Hyperswitch images from the root Dockerfile and push
# them to the GitLab Container Registry, tagged with the commit SHA.
#
# Uses `docker buildx` with a registry-backed layer cache so the cargo-chef
# dependency layer is exported to / imported from the GitLab registry and reused
# across pipelines (the dind daemon is ephemeral, so registry cache is how the
# build cache survives between runs). All four images share the same builder
# stage, so within one pipeline the workspace compiles once and the other three
# reuse it. The per-env release jobs (ci/release-ecr.sh) then promote these
# images to each CDE environment's ECR.
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

CACHE_REF="${CI_REGISTRY_IMAGE}/buildcache"

# A docker-container builder is required for registry cache export/import
# (the default "docker" driver only supports inline cache).
docker buildx inspect hs-builder >/dev/null 2>&1 \
  || docker buildx create --name hs-builder --driver docker-container --use
docker buildx use hs-builder

# name <- "BINARY[,SCHEDULER_FLOW]". router and drainer are distinct binaries;
# producer and consumer are the same `scheduler` binary switched by SCHEDULER_FLOW.
# write_cache: only the first build exports the (shared) builder cache to the
# registry — the rest read it, avoiding redundant multi-hundred-MB exports.
build_and_push() {
  local name="$1" binary="$2" scheduler_flow="${3:-}" write_cache="${4:-false}"
  local image="${CI_REGISTRY_IMAGE}/hyperswitch-${name}:${CI_COMMIT_SHORT_SHA}"

  echo "==> building ${image} (BINARY=${binary}${scheduler_flow:+, SCHEDULER_FLOW=${scheduler_flow}})"
  local args=(
    --build-arg "BINARY=${binary}"
    --build-arg "VERSION_FEATURE_SET=${VERSION_FEATURE_SET}"
    --cache-from "type=registry,ref=${CACHE_REF}"
    --tag "$image"
    --push
  )
  if [ "$write_cache" = "true" ]; then
    args+=(--cache-to "type=registry,ref=${CACHE_REF},mode=max")
  fi
  if [ -n "$scheduler_flow" ]; then
    args+=(--build-arg "SCHEDULER_FLOW=${scheduler_flow}")
  fi
  docker buildx build "${args[@]}" .
}

build_and_push router   router              "" true
build_and_push producer scheduler producer
build_and_push consumer scheduler consumer
build_and_push drainer  drainer

echo "==> done"
