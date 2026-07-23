# Setting up Zoekt for the benchmark suite

Short answer to "is there a Docker version of Zoekt": **yes.** Sourcegraph publishes an official
image at `ghcr.io/sourcegraph/zoekt` that bundles every `zoekt-*` binary (including `zoekt-index`
and the `zoekt` query CLI), plus `git` and `universal-ctags`. `bench/run_zoekt.sh` uses this
image directly — there's nothing to install locally beyond Docker itself.

This doc explains what that script does and how to run/troubleshoot it manually, in case you want
to poke at Zoekt outside the benchmark harness too.

## 1. Pull the image

```bash
docker pull ghcr.io/sourcegraph/zoekt:latest
```

Confirm the binaries you need are in it:

```bash
docker run --rm --entrypoint sh ghcr.io/sourcegraph/zoekt:latest -c 'ls /usr/local/bin'
```

You should see `zoekt`, `zoekt-index`, `zoekt-webserver`, and a handful of others
(`zoekt-git-index`, `zoekt-mirror-*`, etc. — the benchmark only needs `zoekt-index` and `zoekt`).

## 2. Build an index from a local directory

```bash
mkdir -p /path/to/idx
docker run --rm --user root \
  -v /path/to/your/repo:/src \
  -v /path/to/idx:/idx \
  --entrypoint zoekt-index \
  ghcr.io/sourcegraph/zoekt:latest -index /idx /src
```

Notes on the flags:

- **`--user root`** — the image runs as a non-root `zoekt` user by default. That user can't
  write into a bind-mounted host directory it doesn't own, which is almost always the case for a
  freshly created output dir. `--user root` sidesteps that; fine for a throwaway benchmark
  container.
- **`-index /idx /src`** — `zoekt-index`'s own flag is `-index <output-dir>`, followed by one or
  more source directories to scan.

After this, `/path/to/idx` has one or more `*.zoekt` shard files.

## 3. Query the index

```bash
docker run --rm --user root \
  -v /path/to/idx:/idx \
  --entrypoint zoekt \
  ghcr.io/sourcegraph/zoekt:latest -index_dir /idx TODO
```

For a regex query, prefix the pattern with `regex:`:

```bash
ghcr.io/sourcegraph/zoekt:latest -index_dir /idx 'regex:[A-Za-z]+Error'
```

Both of these were verified for real against `ghcr.io/sourcegraph/zoekt:latest` while building
this suite (2026-07-23) — `-index`, `-index_dir`, and the `regex:` prefix all behave as shown.

## 4. The Windows/git-bash path gotchas

Two things bit us building this, in case they bite you too:

1. **`/tmp` doesn't bind-mount correctly on Windows + Docker Desktop.** `-v /tmp/foo:/bar` mounts
   *empty* — Docker Desktop's WSL2 backend doesn't share that path by default, so the container
   sees an empty directory with no error. Use a path under a drive Docker Desktop does share (a
   repo-local directory works reliably) instead of a bare `mktemp -d`/`/tmp/...` path.
2. **git-bash (MSYS) auto-converts `-v` arguments unreliably.** With MSYS's default path
   conversion left on, a `-v <posix-path>:/container/path` argument can get mis-split or have its
   container side rewritten into a garbage Windows path (we saw `Destination` end up as
   `\Program Files\Git\src` — MSYS's fallback heuristic for a bare `/src`-looking string it
   doesn't recognize). The fix: prefix each `docker` invocation with `MSYS_NO_PATHCONV=1` (as a
   command prefix, not a global `export` — exporting it breaks other native Windows tools in the
   same script, like `jq`, that need MSYS's normal conversion to find files by POSIX path), and
   convert the host-side path yourself with `cygpath -m` (turns `/c/repos/foo` into
   `C:/repos/foo`) — `bench/lib.sh`'s `docker_host_path()` does this, no-op on Linux/macOS where
   `cygpath` doesn't exist and no conversion is needed.

`bench/run_zoekt.sh` already does both of these — this section exists so the reasoning isn't a
mystery if you're reading the script or hit the same failure mode running Zoekt some other way.

## 5. What the benchmark actually does

`run_zoekt.sh` starts one long-lived container per corpus (`docker run -d ... sleep infinity`),
runs `docker exec ... zoekt-index` once to build the index (the "cold" number), then runs
`docker exec ... zoekt` repeatedly per query for the "hot" number. It deliberately does **not**
use a fresh `docker run` per query — container startup (~100–300ms) would dominate and make every
query look equally slow regardless of how fast Zoekt's actual lookup is.

## CI

`.github/workflows/benchmark.yml` just does `docker pull ghcr.io/sourcegraph/zoekt:latest` before
calling `bench/run.sh` — no Go toolchain setup needed.
