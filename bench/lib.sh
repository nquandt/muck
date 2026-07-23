#!/usr/bin/env bash
# Shared helpers for bench/run_*.sh. Sourced, not executed directly.

# Milliseconds since epoch — bash's $EPOCHREALTIME (seconds.microseconds) is portable across
# the git-bash/Linux environments this suite targets, unlike `date +%s%N` (no %N on macOS/BSD
# date, and git-bash's date is GNU so it works there, but this is more robust either way).
now_ms() {
  local t="${EPOCHREALTIME/./}"
  echo "${t:0:13}"
}

# Prints the median of its numeric args (milliseconds), integer output.
median_of() {
  python3 -c "
import sys
vals = sorted(float(x) for x in sys.argv[1:])
n = len(vals)
if n == 0:
    print(0)
elif n % 2 == 1:
    print(int(vals[n // 2]))
else:
    print(int((vals[n // 2 - 1] + vals[n // 2]) / 2))
" "$@"
}

# Shallow-clones `url` at `ref` into `dest` if `dest` doesn't already exist. Prints the file
# count (excluding .git) to stdout on success.
clone_corpus() {
  local url="$1" ref="$2" dest="$3"
  if [[ ! -d "${dest}" ]]; then
    git clone --depth 1 --branch "${ref}" --single-branch "${url}" "${dest}" >&2
  fi
  find "${dest}" -type f -not -path '*/.git/*' | wc -l | tr -d ' '
}

# Appends a JSON object (single line) to the shared results file for this run.
emit_result() {
  local json="$1"
  echo "${json}" >>"${BENCH_RESULTS_FILE}"
}
