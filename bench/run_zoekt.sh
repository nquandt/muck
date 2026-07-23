#!/usr/bin/env bash
# Benchmarks Zoekt: cold = `zoekt-index` build time against the corpus. Hot = median `zoekt`
# CLI query latency against the built index shards (queries the on-disk index directly, no
# webserver round-trip — the closest analog to muck's in-memory query path).
#
# Assumes `zoekt-index` and `zoekt` (github.com/sourcegraph/zoekt/cmd/...) are on PATH — see
# bench/Dockerfile for how CI installs them via `go install`.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib.sh"

CORPUS_DIR="$1"
CORPUS_NAME="$2"
QUERIES_FILE="${SCRIPT_DIR}/queries.json"
REPEAT_RUNS="${BENCH_REPEAT_RUNS:-7}"
INDEX_DIR="${BENCH_TMP_DIR:-/tmp}/zoekt-index-${CORPUS_NAME}"

if ! command -v zoekt-index >/dev/null 2>&1 || ! command -v zoekt >/dev/null 2>&1; then
  echo "SKIP: zoekt-index/zoekt not installed" >&2
  exit 0
fi

rm -rf "${INDEX_DIR}"
mkdir -p "${INDEX_DIR}"

t0=$(now_ms)
zoekt-index -index "${INDEX_DIR}" "${CORPUS_DIR}" >&2
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

  durations=()
  for ((r = 0; r < REPEAT_RUNS; r++)); do
    t0=$(now_ms)
    zoekt -index_dir "${INDEX_DIR}" "${q}" >/dev/null 2>&1 || true
    durations+=("$(($(now_ms) - t0))")
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

rm -rf "${INDEX_DIR}"
