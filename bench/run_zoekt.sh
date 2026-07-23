#!/usr/bin/env bash
# Benchmarks Zoekt via the official ghcr.io/sourcegraph/zoekt image (bundles zoekt-index and
# zoekt-webserver — no local Go toolchain needed). Cold = `zoekt-index` build time against the
# corpus. Hot/memory = a real `zoekt-webserver` process, queried over real HTTP — the same way
# `run_muck.sh` measures muck, so the two tools' numbers are actually comparable.
#
# An earlier version of this script measured "hot" via the `zoekt` CLI in a docker-exec loop
# and "memory" via an idle shell right after `zoekt-index` finished — no zoekt-webserver, no
# HTTP round trip, at all. That undersold Zoekt's real per-query cost (no HTTP stack, no
# request routing, an already-hot in-process index) enough to read ~0ms on every query, which
# is not what a real Zoekt deployment (always a webserver) looks like under real traffic.
# Confirmed manually on 2026-07-23 against rails/rails: the CLI-loop method reported 0ms
# across the board; the same corpus against a live zoekt-webserver over HTTP measured
# 6.7-638ms depending on query, and memory read 32MB (vs. the CLI method's 2.7MB idle-shell
# reading). Fixed here so muck-vs-Zoekt comparisons are trustworthy inputs to real decisions,
# not just directionally suggestive.
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
PORT="${ZOEKT_BENCH_PORT:-6070}"

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
  -p "${PORT}:${PORT}" \
  -v "${CORPUS_HOST_PATH}:/src" \
  -v "${INDEX_HOST_PATH}:/idx" \
  --entrypoint sh \
  "${ZOEKT_IMAGE}" -c 'sleep infinity' >/dev/null

t0=$(now_ms)
MSYS_NO_PATHCONV=1 docker exec "${CONTAINER}" zoekt-index -index /idx /src >&2
cold_ms=$(($(now_ms) - t0))
echo "zoekt ${CORPUS_NAME}: index_build=${cold_ms}ms" >&2

# Launch zoekt-webserver as a real long-running process inside the same container (detached
# exec), then wait for it to actually be serving — same "poll until ready" pattern
# run_muck.sh uses for muck's own readiness check, so cold/warm boundaries are defined the
# same way for both tools.
MSYS_NO_PATHCONV=1 docker exec -d "${CONTAINER}" zoekt-webserver -index /idx -listen ":${PORT}"
for _ in $(seq 1 100); do
  if curl -sf "http://127.0.0.1:${PORT}/" >/dev/null 2>&1; then
    break
  fi
  sleep 0.1
done

urlencode() {
  local string="$1" i c encoded=""
  for ((i = 0; i < ${#string}; i++)); do
    c="${string:i:1}"
    case "${c}" in
      [a-zA-Z0-9.~_-]) encoded+="${c}" ;;
      *) printf -v c '%%%02X' "'${c}"; encoded+="${c}" ;;
    esac
  done
  echo "${encoded}"
}

# One real query first, same reasoning as muck's own "build" step warming its index before the
# memory snapshot — without this, the memory reading would reflect an mmap that's been opened
# but never actually faulted into the page cache, understating what a query workload costs.
first_pattern=$(jq -r '.[0].pattern' "${QUERIES_FILE}")
curl -s "http://127.0.0.1:${PORT}/search?q=$(urlencode "${first_pattern}")&num=50" >/dev/null

# Warm-state resource snapshot: a live zoekt-webserver process, not an idle post-index shell —
# this is the real "what does running Zoekt cost" number, comparable to muck's own container
# snapshot in run_muck.sh. Disk is the index shard directory, measured host-side (bind mount).
mem_mb=$(docker_mem_mb "${CONTAINER}")
disk_mb=$(dir_size_mb "${INDEX_DIR}")
echo "zoekt ${CORPUS_NAME}: mem=${mem_mb}MB disk=${disk_mb}MB" >&2
emit_resource "zoekt" "${CORPUS_NAME}" "${mem_mb}" "${disk_mb}" \
  "live zoekt-webserver process, warmed by one query before this snapshot"

query_count=$(jq 'length' "${QUERIES_FILE}")
for ((i = 0; i < query_count; i++)); do
  name=$(jq -r ".[$i].name" "${QUERIES_FILE}")
  pattern=$(jq -r ".[$i].pattern" "${QUERIES_FILE}")
  is_regex=$(jq -r ".[$i].regex" "${QUERIES_FILE}")
  q="${pattern}"
  if [[ "${is_regex}" == "true" ]]; then
    q="regex:${pattern}"
  fi
  enc=$(urlencode "${q}")

  # Real HTTP requests to the live webserver, same curl -w timing method run_muck.sh uses for
  # muck's own /v1/search — this is what makes the two tools' hot-path numbers comparable.
  curl_args=()
  for ((r = 0; r < REPEAT_RUNS; r++)); do
    if ((r > 0)); then curl_args+=(--next); fi
    curl_args+=(-s -o /dev/null -w '%{time_total}\n' "http://127.0.0.1:${PORT}/search?q=${enc}&num=50")
  done
  mapfile -t seconds < <(curl "${curl_args[@]}")
  durations=()
  for s in "${seconds[@]}"; do
    durations+=("$(python3 -c "print(round(${s} * 1000))")")
  done
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
