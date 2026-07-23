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

Both numbers matter for different reasons: cold start is what you pay once per repo/commit;
hot-path is what every subsequent search costs. A tool can lose on cold start and still be the
right choice if it wins hot path by enough and hot path dominates real usage (many searches per
index build) — or vice versa for a CI/one-shot use case.

## Corpora

Three tiers in `corpora.json`:

- `small` — the ripgrep repo itself (~300 files). Fast sanity-check run.
- `medium` (**default**) — django and redis (roughly 1,500–2,700 files each). Realistic size for
  most day-to-day repos.
- `big` — TypeScript and dotnet/runtime (tens of thousands of files each). Opt-in — slow, and
  meant to answer "does this still hold up at scale," not for routine runs.

## Running locally

Requires: `docker`, `git`, `jq`, `curl`, `python3`, `node` (used only to percent-encode file
paths when pushing to muck), and whichever of `rg`/`ag`/`zoekt`+`zoekt-index` you want included —
any tool whose binary isn't found is skipped with a note, not a hard failure.

```bash
./bench/run.sh                                   # medium tier, all tools available on PATH
./bench/run.sh --tier small                      # quick sanity run
./bench/run.sh --tier big --tools muck,zoekt      # skip the CLI tools on the big tier
./bench/run.sh --tools muck,ripgrep               # just these two
```

Results land in `bench/results/<tier>-<timestamp>.jsonl` (raw) and `.md` (summary table).

### Installing Zoekt locally

```bash
go install github.com/sourcegraph/zoekt/cmd/zoekt-index@latest
go install github.com/sourcegraph/zoekt/cmd/zoekt@latest
```

The zoekt CLI's exact flags/query syntax can drift between versions — if `run_zoekt.sh` errors
on `zoekt-index`/`zoekt` invocation, check `zoekt-index -h` / `zoekt -h` against what the script
assumes (`-index <dir> <src>` and `-index_dir <dir> <query>`, with `regex:` as the query prefix
for regex searches) and adjust.

## CI

`.github/workflows/benchmark.yml` runs this on `workflow_dispatch` (manual trigger, pick a tier),
installs ripgrep/ag/zoekt/muck's own toolchain fresh each run, and publishes the markdown summary
as the job summary plus an uploaded artifact with the raw JSON lines.

## Interpreting results

This isn't meant to declare a universal winner — ripgrep/ag with a warm page cache on a
medium-sized repo are legitimately fast, and for a one-shot local search an index's build cost
may never pay for itself. Where an index-backed tool (muck, Zoekt) should win is: many searches
against the same corpus without re-paying the scan cost each time, and searches at a scale where
a full re-scan (even cache-warm) stops being instant. The `big` tier exists to see where that
crossover actually happens.
