#!/usr/bin/env bash
# Benchmarks muck: cold = container start + push all files + build (index) time. Hot = median
# query latency against /v1/search once the index is built and warm in memory.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
source "${SCRIPT_DIR}/lib.sh"

CORPUS_DIR="$1"
CORPUS_NAME="$2"
QUERIES_FILE="${SCRIPT_DIR}/queries.json"
REPEAT_RUNS="${BENCH_REPEAT_RUNS:-7}"
PORT="${MUCK_BENCH_PORT:-7790}"
IMAGE="muck-local:bench"
CONTAINER="muck-bench-${CORPUS_NAME}"
MAX_CONCURRENCY="${BENCH_PUSH_CONCURRENCY:-16}"

docker image inspect "${IMAGE}" >/dev/null 2>&1 || docker build -f "${REPO_ROOT}/Dockerfile.local" -t "${IMAGE}" "${REPO_ROOT}" >&2

docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true
t_container_start=$(now_ms)
docker run -d --name "${CONTAINER}" -p "${PORT}:7777" "${IMAGE}" >/dev/null

# Wait for /health to go 200 — this is the real "container is servable" boundary, not just
# `docker run` returning (the process still has to bind and pass its own readiness check).
for _ in $(seq 1 100); do
  if curl -sf "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done
container_ready_ms=$(($(now_ms) - t_container_start))

# Percent-encodes a string for use as a query-string value (pure bash, no subprocess per
# call — spawning node/python per file here dominated push time and skewed cold-start numbers).
urlencode() {
  local string="$1" i c encoded=""
  for ((i = 0; i < ${#string}; i++)); do
    c="${string:i:1}"
    case "${c}" in
      [a-zA-Z0-9.~_-]) encoded+="${c}" ;;
      '/') encoded+='%2F' ;;
      *) printf -v c '%%%02X' "'${c}"; encoded+="${c}" ;;
    esac
  done
  echo "${encoded}"
}
export -f urlencode

push_file() {
  local file="$1" relpath="$2"
  curl -s -o /dev/null -X PUT "http://127.0.0.1:${PORT}/v1/repos/bench/files?path=$(urlencode "${relpath}")" \
    --data-binary @- <"${file}"
}
export -f push_file
export PORT

t_push_start=$(now_ms)
find "${CORPUS_DIR}" -type f -not -path '*/.git/*' -print0 |
  xargs -0 -P "${MAX_CONCURRENCY}" -I{} bash -c 'push_file "$1" "${1#'"${CORPUS_DIR}"'/}"' _ {}
push_ms=$(($(now_ms) - t_push_start))

t_build_start=$(now_ms)
curl -s -X POST "http://127.0.0.1:${PORT}/v1/repos/bench/build?name=${CORPUS_NAME}&version=bench&org=bench&branch=bench" >/dev/null
build_ms=$(($(now_ms) - t_build_start))

cold_ms=$((container_ready_ms + push_ms + build_ms))
echo "muck ${CORPUS_NAME}: container_ready=${container_ready_ms}ms push=${push_ms}ms build=${build_ms}ms total_cold=${cold_ms}ms" >&2

query_count=$(jq 'length' "${QUERIES_FILE}")
for ((i = 0; i < query_count; i++)); do
  name=$(jq -r ".[$i].name" "${QUERIES_FILE}")
  pattern=$(jq -r ".[$i].pattern" "${QUERIES_FILE}")
  is_regex=$(jq -r ".[$i].regex" "${QUERIES_FILE}")
  body=$(jq -n --arg q "${pattern}" --argjson r "${is_regex}" '{query: $q, regex: $r}')

  durations=()
  for ((r = 0; r < REPEAT_RUNS; r++)); do
    t0=$(now_ms)
    curl -s -X POST "http://127.0.0.1:${PORT}/v1/search" -H 'Content-Type: application/json' -d "${body}" >/dev/null
    durations+=("$(($(now_ms) - t0))")
  done
  hot_ms=$(median_of "${durations[@]}")

  emit_result "$(jq -nc \
    --arg tool "muck" \
    --arg corpus "${CORPUS_NAME}" \
    --arg query "${name}" \
    --argjson cold_ms "${cold_ms}" \
    --argjson hot_ms "${hot_ms}" \
    '{tool: $tool, corpus: $corpus, query: $query, cold_ms: $cold_ms, hot_ms: $hot_ms}')"
done

docker rm -f "${CONTAINER}" >/dev/null 2>&1 || true
