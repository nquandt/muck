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

# Appends a resource-usage row (warm-state memory/disk, measured once per tool+corpus after
# indexing completes and before any queries run) — distinct shape from emit_result's per-query
# rows (no "query" key), so summarize.py splits them into their own table.
emit_resource() {
  local tool="$1" corpus="$2" mem_mb="$3" disk_mb="$4" note="$5"
  emit_result "$(jq -nc \
    --arg tool "${tool}" \
    --arg corpus "${corpus}" \
    --argjson mem_mb "${mem_mb}" \
    --argjson disk_mb "${disk_mb}" \
    --arg note "${note}" \
    '{tool: $tool, corpus: $corpus, resource: true, mem_mb: $mem_mb, disk_mb: $disk_mb, note: (if $note == "" then null else $note end)}')"
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

# Current resident memory of a running container, in MB (integer, rounded). Parses
# `docker stats`'s "<used> / <limit>" MemUsage string (e.g. "45.2MiB / 7.775GiB") — the used
# side is what we want, cgroup-reported RSS+cache for everything in the container.
docker_mem_mb() {
  local container="$1"
  local usage
  usage=$(docker stats --no-stream --format '{{.MemUsage}}' "${container}" 2>/dev/null | awk '{print $1}')
  python3 -c "
import re, sys
m = re.match(r'([\d.]+)\s*([A-Za-z]+)', sys.argv[1] or '')
if not m:
    print(0)
    sys.exit()
val, unit = float(m.group(1)), m.group(2).lower()
mult = {'b': 1/1e6, 'kb': 1/1000, 'kib': 1/976.5625, 'mb': 1, 'mib': 1.048576, 'gb': 1000, 'gib': 1073.741824}
print(round(val * mult.get(unit, 1), 1))
" "${usage}"
}

# Total size of a directory on the host, in MB (integer, rounded) — used to measure an index's
# on-disk footprint (e.g. Zoekt's shard files) directly from the host side, since it's a bind
# mount rather than part of any container's own writable layer.
dir_size_mb() {
  local path="$1"
  local kb
  kb=$(du -sk "${path}" 2>/dev/null | awk '{print $1}')
  python3 -c "print(round((${kb:-0}) / 1024, 1))"
}

# Writable-layer disk usage of a container, in MB — what that container itself has written to
# disk, excluding the (shared, read-only) base image layers. `docker ps -s`'s Size column looks
# like "4.1kB (virtual 104MB)"; we want the part before "(virtual", not the virtual total.
docker_writable_layer_mb() {
  local container="$1"
  local size_str
  size_str=$(docker ps -s --filter "name=^${container}\$" --format '{{.Size}}' 2>/dev/null | sed 's/ (virtual.*//')
  python3 -c "
import re, sys
m = re.match(r'([\d.]+)\s*([A-Za-z]+)', sys.argv[1] or '')
if not m:
    print(0)
    sys.exit()
val, unit = float(m.group(1)), m.group(2).lower()
mult = {'b': 1/1e6, 'kb': 1/1000, 'kib': 1/976.5625, 'mb': 1, 'mib': 1.048576, 'gb': 1000, 'gib': 1073.741824}
print(round(val * mult.get(unit, 1), 1))
" "${size_str}"
}
