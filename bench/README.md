# muck benchmarks

Compares muck against [Zoekt](https://github.com/sourcegraph/zoekt) (the closest architectural
peer — persistent trigram index + query server), and against [ripgrep](https://github.com/BurntSushi/ripgrep)
and [ag](https://github.com/ggreer/the_silver_searcher) (the dominant no-index CLI baselines).

## What's measured

- **Cold start**: for muck and Zoekt, this is container/process start + the one-time cost of
  building an index from the corpus (push-all-files + build, or `zoekt-index`). For ripgrep/ag,
  which have no index, "cold" is the first scan of the corpus (page cache not yet warm) — there's
  no equivalent "build" step to measure, so the first-run number stands in for it.
- **Hot path**: median query latency over `BENCH_REPEAT_RUNS` (default 7) repeated runs of the
  same query, after the index (or page cache, for ripgrep/ag) is warm. This is the number that
  actually answers "does an index help" — ripgrep/ag still re-scan every file on every "hot" run,
  they just do it against warm disk cache instead of cold.
- **Warm-state memory & disk**: taken once per tool+corpus right after indexing finishes and
  before any queries run — muck and Zoekt via `docker stats` (whole-container memory) and either
  the container's own writable disk layer (muck) or the index shard directory measured host-side
  (Zoekt, since it's a bind mount). ripgrep/ag report `mem: —`/`disk: 0` with a note explaining
  why: they have no persistent process, so there's nothing to measure — that absence (zero
  resident cost, but a full re-scan every single search) is itself the relevant data point.
  **Caveat**: the Zoekt container in this suite only runs one-shot `zoekt-index`/`zoekt` CLI
  calls, not a long-running `zoekt-webserver` — so its memory number reflects an idle shell with
  index shards sitting on disk, not the resident/mmap'd footprint a real always-on Zoekt
  deployment would show. muck's number is the real thing: muck's own container *is* the query
  server, holding every pushed file's bytes in memory for as long as it runs (see
  `src/store.rs`) — so muck's and Zoekt's memory numbers aren't measuring quite the same kind of
  "warm," and shouldn't be read as strictly apples-to-apples without that in mind.

All timings are millisecond-resolution. A `0` in the hot-path table isn't a bug — it means the
query genuinely resolved in under a millisecond (index-backed lookups against a small corpus
routinely do); it's evidence of the index paying off, not evidence something broke.

Both numbers matter for different reasons: cold start is what you pay once per repo/commit;
hot-path is what every subsequent search costs. A tool can lose on cold start and still be the
right choice if it wins hot path by enough and hot path dominates real usage (many searches per
index build) — or vice versa for a CI/one-shot use case.

## Corpora

Three tiers in `corpora.json`:

- `small` — the ripgrep repo itself (~300 files). Fast sanity-check run.
- `medium` (**default**) — django, redis, and rails (roughly 1,500–5,000 files each). Realistic
  size for most day-to-day repos.
- `big` — TypeScript and dotnet/runtime (tens of thousands of files each). Opt-in — slow, and
  meant to answer "does this still hold up at scale," not for routine runs.

## Running locally

Requires: `docker`, `git`, `jq`, `curl`, `python3`, and whichever of `rg`/`ag` you want included
locally — Zoekt runs via its official Docker image (see below), no separate install needed. Any
tool whose binary/image isn't found is skipped with a note, not a hard failure. See
[ZOEKT_SETUP.md](ZOEKT_SETUP.md) for the full walkthrough if you're setting this up fresh.

```bash
./bench/run.sh                                   # medium tier, all tools available on PATH
./bench/run.sh --tier small                      # quick sanity run
./bench/run.sh --tier big --tools muck,zoekt      # skip the CLI tools on the big tier
./bench/run.sh --tools muck,ripgrep               # just these two
```

Results land in `bench/results/<tier>-<timestamp>.jsonl` (raw) and `.md` (summary table).

### Zoekt

`run_zoekt.sh` runs Zoekt via `docker`, using the official `ghcr.io/sourcegraph/zoekt` image —
nothing to install beyond Docker itself. See [ZOEKT_SETUP.md](ZOEKT_SETUP.md) if you want to
understand or troubleshoot how that works.

## CI

`.github/workflows/benchmark.yml` runs this on `workflow_dispatch` (manual trigger, pick a tier),
installs ripgrep/ag/zoekt/muck's own toolchain fresh each run, and publishes the markdown summary
as the job summary plus an uploaded artifact with the raw JSON lines.

## A Windows-specific caveat on muck's cold-start number

On a real run against rails (~5,000 files) on Windows + Docker Desktop, muck's cold start was
dominated by `push` time — ~150s of the ~170s total, pushing files one at a time via individual
`curl` calls from git-bash (muck's own `build`/indexing step was ~15s, the same order as Zoekt's
index build). That per-file push cost (~30ms/file) lines up with the same Docker-Desktop-VM
network-boundary overhead documented in `ZOEKT_SETUP.md` and fixed in the hot-path timing — it's
a property of pushing many small HTTP requests through that boundary on this platform, not of
muck's own indexing speed. This hasn't been verified on Linux (e.g. in CI), where that per-call
overhead is expected to be much smaller since there's no VM boundary to cross. Read muck's
cold-start number with that in mind until a Linux run confirms (or refutes) it.

## Interpreting results

This isn't meant to declare a universal winner — ripgrep/ag with a warm page cache on a
medium-sized repo are legitimately fast, and for a one-shot local search an index's build cost
may never pay for itself. Where an index-backed tool (muck, Zoekt) should win is: many searches
against the same corpus without re-paying the scan cost each time, and searches at a scale where
a full re-scan (even cache-warm) stops being instant. The `big` tier exists to see where that
crossover actually happens.
