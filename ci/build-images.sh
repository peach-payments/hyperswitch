#!/usr/bin/env bash
#
# Build the four deployable Hyperswitch images from the root Dockerfile and push
# them to the current environment's AWS ECR.
#
# All four images share an identical builder stage, so building them against a
# single dind daemon compiles the Rust workspace once and reuses the cached
# layer for the remaining three.
#
# Expects the caller (before_script in .gitlab-ci.yml) to have already:
#   * assumed the environment role (AWS_* creds exported), and
#   * logged docker in to ECR and exported ECR_REGISTRY.
#
# Driven by:
#   ECR_REGISTRY          registry host (derived from the assumed identity)
#   AWS_DEFAULT_REGION    ECR region (defaults to eu-west-1)
#   VERSION_FEATURE_SET   cargo feature set (v1 / v2), defaults to v1
#   CI_COMMIT_SHORT_SHA   short commit sha (always used as a tag)
#   CI_COMMIT_BRANCH      current branch (adds "latest" when it is the default)
#   CI_DEFAULT_BRANCH     default branch
#   CI_COMMIT_TAG         git tag name (added as a tag on tagged pipelines)

set -euo pipefail

VERSION_FEATURE_SET="${VERSION_FEATURE_SET:-v1}"
AWS_DEFAULT_REGION="${AWS_DEFAULT_REGION:-eu-west-1}"

# Fall back to deriving the registry if it wasn't exported by before_script.
if [ -z "${ECR_REGISTRY:-}" ]; then
  ACCOUNT_ID="$(aws sts get-caller-identity --query Account --output text)"
  ECR_REGISTRY="${ACCOUNT_ID}.dkr.ecr.${AWS_DEFAULT_REGION}.amazonaws.com"
fi
echo "Using ECR registry: ${ECR_REGISTRY}"

# Create the ECR repo if it does not exist (MUTABLE tags, scan-on-push),
# matching the CDE house convention.
ensure_repo() {
  local repo="$1"
  if ! aws ecr describe-repositories \
        --repository-names "$repo" \
        --region "$AWS_DEFAULT_REGION" >/dev/null 2>&1; then
    echo "ECR repo ${repo} not found; creating (MUTABLE, scan-on-push)."
    aws ecr create-repository \
      --repository-name "$repo" \
      --image-tag-mutability MUTABLE \
      --image-scanning-configuration scanOnPush=true \
      --region "$AWS_DEFAULT_REGION" >/dev/null
  fi
}

# repo <- "BINARY[,SCHEDULER_FLOW]". router and drainer are distinct binaries;
# producer and consumer are the same `scheduler` binary switched by SCHEDULER_FLOW.
build_and_push() {
  local repo="$1" binary="$2" scheduler_flow="${3:-}"
  local local_tag="${repo}:build"

  echo "==> building ${repo} (BINARY=${binary}${scheduler_flow:+, SCHEDULER_FLOW=${scheduler_flow}})"
  local args=(
    --build-arg "BINARY=${binary}"
    --build-arg "VERSION_FEATURE_SET=${VERSION_FEATURE_SET}"
  )
  if [ -n "$scheduler_flow" ]; then
    args+=(--build-arg "SCHEDULER_FLOW=${scheduler_flow}")
  fi
  docker build "${args[@]}" -t "$local_tag" .

  ensure_repo "$repo"

  # Tags: always the short sha, the git tag on tagged pipelines, and "latest"
  # on the default branch.
  local tags=("${CI_COMMIT_SHORT_SHA:?CI_COMMIT_SHORT_SHA must be set}")
  if [ -n "${CI_COMMIT_TAG:-}" ]; then
    tags+=("${CI_COMMIT_TAG}")
  fi
  if [ -n "${CI_COMMIT_BRANCH:-}" ] && [ "${CI_COMMIT_BRANCH:-}" = "${CI_DEFAULT_BRANCH:-}" ]; then
    tags+=("latest")
  fi

  for tag in "${tags[@]}"; do
    local remote="${ECR_REGISTRY}/${repo}:${tag}"
    docker tag "$local_tag" "$remote"
    echo "==> pushing ${remote}"
    docker push "$remote"
  done
}

build_and_push peach/hyperswitch-router   router
build_and_push peach/hyperswitch-producer scheduler producer
build_and_push peach/hyperswitch-consumer scheduler consumer
build_and_push peach/hyperswitch-drainer  drainer

echo "==> done"
