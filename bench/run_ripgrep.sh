#!/usr/bin/env bash
# Benchmarks ripgrep against a corpus. No persistent index exists for ripgrep — "cold" here
# means the first scan (page cache not yet warmed for these files), "hot" means the median of
# REPEAT_RUNS subsequent scans (page cache warm) — the fairest available proxy for "steady
# state" since there's no index to build/query separately.
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib.sh"

CORPUS_DIR="$1"
CORPUS_NAME="$2"
QUERIES_FILE="${SCRIPT_DIR}/queries.json"
REPEAT_RUNS="${BENCH_REPEAT_RUNS:-7}"

if ! command -v rg >/dev/null 2>&1; then
  echo "SKIP: ripgrep (rg) not installed" >&2
  exit 0
fi

query_count=$(jq 'length' "${QUERIES_FILE}")
for ((i = 0; i < query_count; i++)); do
  name=$(jq -r ".[$i].name" "${QUERIES_FILE}")
  pattern=$(jq -r ".[$i].pattern" "${QUERIES_FILE}")
  is_regex=$(jq -r ".[$i].regex" "${QUERIES_FILE}")
  flags=(-S)
  if [[ "${is_regex}" != "true" ]]; then
    flags+=(-F)
  fi

  # Cold: first run against this pattern.
  t0=$(now_ms)
  rg "${flags[@]}" -e "${pattern}" "${CORPUS_DIR}" >/dev/null 2>&1 || true
  cold_ms=$(($(now_ms) - t0))

  # Hot: repeat, median.
  durations=()
  for ((r = 0; r < REPEAT_RUNS; r++)); do
    t0=$(now_ms)
    rg "${flags[@]}" -e "${pattern}" "${CORPUS_DIR}" >/dev/null 2>&1 || true
    durations+=("$(($(now_ms) - t0))")
  done
  hot_ms=$(median_of "${durations[@]}")

  emit_result "$(jq -nc \
    --arg tool "ripgrep" \
    --arg corpus "${CORPUS_NAME}" \
    --arg query "${name}" \
    --argjson cold_ms "${cold_ms}" \
    --argjson hot_ms "${hot_ms}" \
    '{tool: $tool, corpus: $corpus, query: $query, cold_ms: $cold_ms, hot_ms: $hot_ms}')"
done
