#!/usr/bin/env bash
#
# Promote the images built by ci/build-images.sh from the GitLab Container
# Registry to the current CDE environment's ECR: pull the SHA-tagged image,
# ensure the ECR repo exists, retag and push (SHA, plus latest on the default
# branch and the tag name on tagged pipelines).
#
# Expects the caller (before_script) to have already:
#   * logged docker in to $CI_REGISTRY (to pull), and
#   * assumed the environment role and logged docker in to ECR, exporting
#     ECR_REGISTRY.
#
# Driven by:
#   CI_REGISTRY_IMAGE     GitLab registry base for this project
#   CI_COMMIT_SHORT_SHA   short commit sha (source image tag)
#   ECR_REGISTRY          target ECR registry host (from the assumed identity)
#   AWS_DEFAULT_REGION    ECR region (defaults to eu-west-1)
#   CI_COMMIT_BRANCH / CI_DEFAULT_BRANCH / CI_COMMIT_TAG  drive the extra tags

set -euo pipefail

AWS_DEFAULT_REGION="${AWS_DEFAULT_REGION:-eu-west-1}"
: "${CI_REGISTRY_IMAGE:?}"
: "${CI_COMMIT_SHORT_SHA:?}"
: "${ECR_REGISTRY:?}"

# Create the ECR repo if it does not exist (MUTABLE tags, scan-on-push).
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

promote() {
  local name="$1"
  local src="${CI_REGISTRY_IMAGE}/hyperswitch-${name}:${CI_COMMIT_SHORT_SHA}"
  local repo="peach/hyperswitch-${name}"

  echo "==> pulling ${src}"
  docker pull "$src"

  ensure_repo "$repo"

  # Tags: always the short sha, the git tag on tagged pipelines, and "latest"
  # on the default branch.
  local tags=("${CI_COMMIT_SHORT_SHA}")
  if [ -n "${CI_COMMIT_TAG:-}" ]; then
    tags+=("${CI_COMMIT_TAG}")
  fi
  if [ -n "${CI_COMMIT_BRANCH:-}" ] && [ "${CI_COMMIT_BRANCH:-}" = "${CI_DEFAULT_BRANCH:-}" ]; then
    tags+=("latest")
  fi

  for tag in "${tags[@]}"; do
    local dst="${ECR_REGISTRY}/${repo}:${tag}"
    docker tag "$src" "$dst"
    echo "==> pushing ${dst}"
    docker push "$dst"
  done
}

promote router
promote producer
promote consumer
promote drainer

echo "==> done"
