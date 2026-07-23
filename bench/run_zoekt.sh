#!/usr/bin/env bash
# Benchmarks Zoekt via the official ghcr.io/sourcegraph/zoekt image (bundles zoekt-index and
# the `zoekt` query CLI — no local Go toolchain needed). Cold = `zoekt-index` build time
# against the corpus. Hot = median `zoekt` CLI query latency against the built index shards.
#
# Runs one long-lived container for the whole corpus rather than a fresh `docker run` per
# query — that would pay ~100-300ms of container startup every time. Hot-path repeats for a
# given query are timed with ONE `docker exec` running a shell loop that calls `zoekt` N times
# and times each with the container's own `date`, rather than one `docker exec` per repeat —
# `docker exec` itself has real per-call overhead crossing the Docker Desktop VM boundary
# (measured ~350-600ms on Windows), which would swamp the actual `zoekt` process cost and make
# "hot" measure exec overhead instead of query latency.
#
# Verified against ghcr.io/sourcegraph/zoekt:latest on 2026-07-23: `zoekt-index -index <dir>
# <src>`, `zoekt -index_dir <dir> <query>`, and `regex:<pattern>` for regex queries all behave
# as this script assumes. See bench/ZOEKT_SETUP.md for the full walkthrough.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib.sh"

CORPUS_DIR="$1"
CORPUS_NAME="$2"
QUERIES_FILE="${SCRIPT_DIR}/queries.json"
REPEAT_RUNS="${BENCH_REPEAT_RUNS:-7}"
ZOEKT_IMAGE="${ZOEKT_IMAGE:-ghcr.io/sourcegraph/zoekt:latest}"
INDEX_DIR="${BENCH_TMP_DIR:-${SCRIPT_DIR}}/zoekt-index-${CORPUS_NAME}"
CONTAINER="zoekt-bench-${CORPUS_NAME}"

if ! command -v docker >/dev/null 2>&1; then
  echo "SKIP: docker not available (needed to run ${ZOEKT_IMAGE})" >&2
  exit 0
fi
if ! docker image inspect "${ZOEKT_IMAGE}" >/dev/null 2>&1 && ! docker pull "${ZOEKT_IMAGE}" >&2; then
  echo "SKIP: could not pull ${ZOEKT_IMAGE}" >&2
  exit 0
fi

rm -rf "${INDEX_DIR}"
mkdir -p "${INDEX_DIR}"
MSYS_NO_PATHCONV=1 docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true

CORPUS_HOST_PATH="$(docker_host_path "${CORPUS_DIR}")"
INDEX_HOST_PATH="$(docker_host_path "${INDEX_DIR}")"

# `--user root`: the image runs as a non-root `zoekt` user by default, which can't write into
# a freshly bind-mounted host directory owned by the host user.
MSYS_NO_PATHCONV=1 docker run -d --rm --name "${CONTAINER}" --user root \
  -v "${CORPUS_HOST_PATH}:/src" \
  -v "${INDEX_HOST_PATH}:/idx" \
  --entrypoint sh \
  "${ZOEKT_IMAGE}" -c 'sleep infinity' >/dev/null

t0=$(now_ms)
MSYS_NO_PATHCONV=1 docker exec "${CONTAINER}" zoekt-index -index /idx /src >&2
cold_ms=$(($(now_ms) - t0))
echo "zoekt ${CORPUS_NAME}: index_build=${cold_ms}ms" >&2

query_count=$(jq 'length' "${QUERIES_FILE}")
for ((i = 0; i < query_count; i++)); do
  name=$(jq -r ".[$i].name" "${QUERIES_FILE}")
  pattern=$(jq -r ".[$i].pattern" "${QUERIES_FILE}")
  is_regex=$(jq -r ".[$i].regex" "${QUERIES_FILE}")
  q="${pattern}"
  if [[ "${is_regex}" == "true" ]]; then
    q="regex:${pattern}"
  fi

  # One `docker exec` runs a loop of REPEAT_RUNS `zoekt` invocations and prints one
  # millisecond duration per line, timed with the container's own `date +%s%N` — keeps
  # docker-exec's own overhead out of the per-query numbers (see header comment).
  loop_script="i=0; while [ \$i -lt ${REPEAT_RUNS} ]; do t0=\$(date +%s%N); zoekt -index_dir /idx '${q}' >/dev/null 2>&1; t1=\$(date +%s%N); echo \$(( (t1 - t0) / 1000000 )); i=\$((i + 1)); done"
  mapfile -t durations < <(MSYS_NO_PATHCONV=1 docker exec "${CONTAINER}" sh -c "${loop_script}")
  hot_ms=$(median_of "${durations[@]}")

  emit_result "$(jq -nc \
    --arg tool "zoekt" \
    --arg corpus "${CORPUS_NAME}" \
    --arg query "${name}" \
    --argjson cold_ms "${cold_ms}" \
    --argjson hot_ms "${hot_ms}" \
    '{tool: $tool, corpus: $corpus, query: $query, cold_ms: $cold_ms, hot_ms: $hot_ms}')"
done

MSYS_NO_PATHCONV=1 docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true
rm -rf "${INDEX_DIR}"
