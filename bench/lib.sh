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

# Converts a path to a form safe to use as the host side of `docker run/exec -v`. On git-bash
# (MSYS), `cygpath -m` turns a POSIX-style path (e.g. `/c/repos/foo`) into the drive-letter form
# Docker Desktop expects (`C:/repos/foo`) — without this, MSYS's own automatic path conversion
# for `-v` arguments is unreliable (it can mis-split the `host:container` pair, or rewrite a
# bare `/container/path` positional arg into a nonsense host path). Callers should also prefix
# each `docker` invocation with `MSYS_NO_PATHCONV=1` (scoped to that command only — exporting it
# for the whole script breaks other native Windows tools like `jq` that need MSYS's normal path
# conversion) so MSYS doesn't try to "help" a second time. On Linux/macOS, `cygpath` doesn't
# exist and the input path is already correct as-is.
docker_host_path() {
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -m "$1"
  else
    echo "$1"
  fi
}
