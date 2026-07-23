#!/usr/bin/env bash
# Orchestrates the full benchmark suite: clones each repo in the chosen corpus tier, then runs
# every requested tool's runner against it, collecting results into a single JSON-lines file
# that bench/summarize.py turns into a markdown table.
#
# Usage:
#   ./bench/run.sh [--tier small|medium|big] [--tools muck,zoekt,ripgrep,ag] [--repo NAME] [--out DIR]
#
# --repo restricts to a single named repo within the tier (matches corpora.json's "name") —
# useful to re-run just one corpus instead of the whole tier.
#
# Env vars:
#   BENCH_REPEAT_RUNS   how many times to repeat each hot-path query (default 7, median taken)
#   BENCH_WORKDIR       where corpora get cloned (default a temp dir, reused across runs if set)
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib.sh"

TIER="medium"
TOOLS="muck,zoekt,ripgrep,ag"
OUT_DIR="${SCRIPT_DIR}/results"
REPO_FILTER=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tier) TIER="$2"; shift 2 ;;
    --tools) TOOLS="$2"; shift 2 ;;
    --out) OUT_DIR="$2"; shift 2 ;;
    --repo) REPO_FILTER="$2"; shift 2 ;;
    *) echo "Unknown arg: $1" >&2; exit 1 ;;
  esac
done

mkdir -p "${OUT_DIR}"
# Defaults under bench/, not a bare `mktemp -d` (usually /tmp) — on Windows + Docker Desktop,
# bind-mounting a path under /tmp into a container silently mounts empty (that path isn't in
# Docker Desktop's shared-drive list), which the Zoekt runner depends on working correctly.
# A repo-local dir is reliably mountable on every platform this suite runs on.
WORKDIR="${BENCH_WORKDIR:-${SCRIPT_DIR}/.workdir}"
mkdir -p "${WORKDIR}"
export BENCH_TMP_DIR="${WORKDIR}"

RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
export BENCH_RESULTS_FILE="${OUT_DIR}/${TIER}-${RUN_ID}.jsonl"
: >"${BENCH_RESULTS_FILE}"

echo "Tier: ${TIER} | Tools: ${TOOLS} | Results: ${BENCH_RESULTS_FILE}" >&2

repo_count=$(jq ".\"${TIER}\" | length" "${SCRIPT_DIR}/corpora.json")
for ((i = 0; i < repo_count; i++)); do
  name=$(jq -r ".\"${TIER}\"[$i].name" "${SCRIPT_DIR}/corpora.json")
  if [[ -n "${REPO_FILTER}" && "${name}" != "${REPO_FILTER}" ]]; then
    continue
  fi
  url=$(jq -r ".\"${TIER}\"[$i].url" "${SCRIPT_DIR}/corpora.json")
  ref=$(jq -r ".\"${TIER}\"[$i].ref" "${SCRIPT_DIR}/corpora.json")
  dest="${WORKDIR}/${name}"

  echo "=== ${name} (${url}@${ref}) ===" >&2
  file_count=$(clone_corpus "${url}" "${ref}" "${dest}")
  echo "  ${file_count} files" >&2

  IFS=',' read -ra tool_list <<<"${TOOLS}"
  for tool in "${tool_list[@]}"; do
    echo "  -> ${tool}" >&2
    case "${tool}" in
      muck) "${SCRIPT_DIR}/run_muck.sh" "${dest}" "${name}" ;;
      zoekt) "${SCRIPT_DIR}/run_zoekt.sh" "${dest}" "${name}" ;;
      ripgrep | rg) "${SCRIPT_DIR}/run_ripgrep.sh" "${dest}" "${name}" ;;
      ag) "${SCRIPT_DIR}/run_ag.sh" "${dest}" "${name}" ;;
      *) echo "  unknown tool: ${tool}, skipping" >&2 ;;
    esac
  done
done

echo "Done. Raw results: ${BENCH_RESULTS_FILE}" >&2
python3 "${SCRIPT_DIR}/summarize.py" "${BENCH_RESULTS_FILE}" >"${OUT_DIR}/${TIER}-${RUN_ID}.md"
echo "Summary: ${OUT_DIR}/${TIER}-${RUN_ID}.md" >&2
