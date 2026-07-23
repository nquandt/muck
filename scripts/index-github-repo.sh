#!/usr/bin/env bash
# Clones a GitHub repo and indexes it into a running muck instance.
#
# ---------------------------------------------------------------------------
# Setting up a fresh muck (Docker)
# ---------------------------------------------------------------------------
#   cd muck
#   docker build -t muck:local .
#   docker run -d --name muck -p 7777:7777 muck:local
#
# Confirm it's up:
#   curl -s http://localhost:7777/health
#   # => {"status":"ok","version":"0.2.0"}
#
# muck is purely in-memory (no volumes, no config needed) — stopping the
# container drops everything it has indexed. To reset an existing instance, just
# restart it:
#   docker restart muck
# ---------------------------------------------------------------------------
#
# Usage:
#   ./index-github-repo.sh <github-repo-url-or-org/repo> [branch] [muck-base-url] [repo-id]
#
# Examples:
#   ./index-github-repo.sh https://github.com/BurntSushi/ripgrep
#   ./index-github-repo.sh momokun7/xgrep main
#   ./index-github-repo.sh momokun7/xgrep main http://localhost:7777 xgrep-upstream
#
# Env var overrides (same as positional args, positional args win if both given):
#   MUCK_BASE_URL   default: http://localhost:7777
#   MAX_CONCURRENCY  default: 8   (parallel file pushes)

set -euo pipefail

usage() {
  grep '^#' "$0" | sed -e 's/^#!.*//' -e 's/^# \{0,1\}//'
  exit 1
}

REPO_ARG="${1:-}"
BRANCH="${2:-}"
MUCK_BASE_URL="${3:-${MUCK_BASE_URL:-http://localhost:7777}}"
REPO_ID_OVERRIDE="${4:-}"
MAX_CONCURRENCY="${MAX_CONCURRENCY:-8}"

if [[ -z "${REPO_ARG}" ]]; then
  usage
fi

# Accept "org/repo" shorthand as well as a full URL.
if [[ "${REPO_ARG}" != http*://* ]]; then
  REPO_URL="https://github.com/${REPO_ARG}"
else
  REPO_URL="${REPO_ARG}"
fi

REPO_NAME="$(basename "${REPO_URL}" .git)"
REPO_ID="${REPO_ID_OVERRIDE:-${REPO_NAME}}"

# "org" is meant as the GitHub-org equivalent — the unit repos are actually scoped/grouped
# under. For GitHub that's the owner segment: https://github.com/<org>/<repo>. For Azure
# DevOps, repos are scoped to a *project*, not the company-level organization — a clone URL
# looks like https://dev.azure.com/<company-org>/<project>/_git/<repo> (or
# https://<company-org>@dev.azure.com/<company-org>/<project>/_git/<repo>) — so the
# GitHub-equivalent "org" there is the segment right before `_git`, not the first path
# segment (which would be the ADO company org, a level too high).
if [[ "${REPO_URL}" == *_git/* ]]; then
  ORG="$(echo "${REPO_URL}" | sed -E 's#.*/([^/]+)/_git/.*#\1#')"
else
  ORG="$(echo "${REPO_URL}" | sed -E 's#^https?://[^/]+/([^/]+)/.*#\1#')"
fi

for cmd in git curl file; do
  if ! command -v "${cmd}" >/dev/null 2>&1; then
    echo "Missing required command: ${cmd}" >&2
    exit 1
  fi
done

echo "Checking muck at ${MUCK_BASE_URL} ..."
if ! curl -sf "${MUCK_BASE_URL}/health" >/dev/null; then
  cat >&2 <<EOF
muck is not reachable at ${MUCK_BASE_URL}.

Start one first:
  cd muck
  docker build -t muck:local .
  docker run -d --name muck -p 7777:7777 muck:local
EOF
  exit 1
fi

WORKDIR="$(mktemp -d)"
cleanup() {
  if [[ -d "${WORKDIR}" ]]; then
    echo "Cleaning up temporary clone directory: ${WORKDIR}" >&2
    rm -rf "${WORKDIR}"
  fi
}
trap cleanup EXIT

CLONE_DIR="${WORKDIR}/${REPO_NAME}"

echo "Cloning ${REPO_URL} (${BRANCH:-default branch}, depth 1) ..."
CLONE_ARGS=(--depth 1)
if [[ -n "${BRANCH}" ]]; then
  CLONE_ARGS+=(--branch "${BRANCH}" --single-branch)
fi
git clone "${CLONE_ARGS[@]}" "${REPO_URL}" "${CLONE_DIR}"

COMMIT_SHA="$(git -C "${CLONE_DIR}" rev-parse HEAD)"
# The actual checked-out branch name (not just whatever the caller passed in, which may be
# empty) — falls back to "HEAD" if detached (e.g. a tag/sha was requested instead of a branch).
BRANCH_NAME="$(git -C "${CLONE_DIR}" rev-parse --abbrev-ref HEAD)"
echo "Cloned at commit ${COMMIT_SHA} (branch: ${BRANCH_NAME}, org: ${ORG})"

# Percent-encodes a string for use as a query-string value (pure bash, no deps).
urlencode() {
  local string="$1" i c
  local length=${#string}
  local encoded=""
  for (( i = 0; i < length; i++ )); do
    c="${string:i:1}"
    case "${c}" in
      [a-zA-Z0-9.~_-]) encoded+="${c}" ;;
      *) printf -v c '%%%02X' "'${c}"; encoded+="${c}" ;;
    esac
  done
  echo "${encoded}"
}
export -f urlencode

push_file() {
  local repo_dir="$1" repo_id="$2" base_url="$3" file="$4"
  local relpath="${file#"${repo_dir}"/}"

  # Skip binaries — muck only indexes text content.
  if file -b --mime-encoding "${file}" | grep -q '^binary$'; then
    return 0
  fi

  local encoded_path
  encoded_path="$(urlencode "${relpath}")"

  # --data-binary @file applies curl's own path-globbing to the filename (interprets
  # literal [ ] { } characters in the path), which breaks on real-world paths like
  # "[slug]/index.tsx". Piping via stdin (@-) reads the file as-is with no glob parsing.
  local status
  status="$(curl -s -o /dev/null -w '%{http_code}' \
    -X PUT "${base_url}/v1/repos/${repo_id}/files?path=${encoded_path}" \
    --data-binary @- < "${file}")"

  if [[ "${status}" != "204" ]]; then
    echo "  WARN: failed to push ${relpath} (HTTP ${status})" >&2
  fi
}
export -f push_file

FILE_COUNT="$(find "${CLONE_DIR}" -type f -not -path '*/.git/*' | wc -l | tr -d ' ')"
echo "Pushing ${FILE_COUNT} files to ${MUCK_BASE_URL} (repoId=${REPO_ID}, up to ${MAX_CONCURRENCY} at a time) ..."
export CLONE_DIR REPO_ID MUCK_BASE_URL
find "${CLONE_DIR}" -type f -not -path '*/.git/*' -print0 \
  | xargs -0 -P "${MAX_CONCURRENCY}" -n 128 bash -c 'for file do push_file "$CLONE_DIR" "$REPO_ID" "$MUCK_BASE_URL" "$file"; done' _

echo "Building index (name=${REPO_NAME}, version=${COMMIT_SHA:0:12}, org=${ORG}, branch=${BRANCH_NAME}) ..."
ENCODED_NAME="$(urlencode "${REPO_NAME}")"
ENCODED_ORG="$(urlencode "${ORG}")"
ENCODED_BRANCH="$(urlencode "${BRANCH_NAME}")"

# A single "Open in GitHub" link template — {org}/{repoName}/{branch}/{path}/{line} get
# substituted client-side by the muck UI. Muck itself treats this as an opaque string.
GITHUB_LINK_JSON="[{\"name\":\"Open in GitHub\",\"urlTemplate\":\"${REPO_URL%.git}/blob/{branch}/{path}#L{line}\"}]"
ENCODED_LINKS="$(urlencode "${GITHUB_LINK_JSON}")"

BUILD_STATUS="$(curl -s -o /dev/null -w '%{http_code}' \
  -X POST "${MUCK_BASE_URL}/v1/repos/${REPO_ID}/build?name=${ENCODED_NAME}&version=${COMMIT_SHA}&org=${ENCODED_ORG}&branch=${ENCODED_BRANCH}&links=${ENCODED_LINKS}")"

if [[ "${BUILD_STATUS}" != "200" ]]; then
  echo "Build failed (HTTP ${BUILD_STATUS})" >&2
  exit 1
fi

echo "Done. Indexed ~${FILE_COUNT} files from ${REPO_NAME}@${COMMIT_SHA:0:12} as repoId '${REPO_ID}'."
echo
echo "Try it:"
echo "  curl -s ${MUCK_BASE_URL}/v1/index/status"
echo "  curl -s -X POST ${MUCK_BASE_URL}/v1/search -H 'Content-Type: application/json' -d '{\"query\":\"TODO\"}'"
