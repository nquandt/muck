# muck

A fast code search server with an embedded GitHub-code-search-style UI. Push files from any
repo (GitHub, Azure DevOps, or a local checkout) into a trigram index and search across them
over HTTP — no external dependencies, no separate indexing step to stand up. The trigram
index itself is pure in-memory, using a compact flat-buffer encoding (see [Storage](#storage))
rather than one heap allocation per distinct trigram; pushed file content lives in
`mmap`-backed on-disk shard files rather than fully resident in the process heap, so most of a
large corpus's memory footprint is handled by the OS page cache instead. An optional
local-disk backup can also be enabled to survive restarts (see [Persistence](#persistence)).

## Why

Sometimes you just want fast full-text/regex search across a pile of repos — locally,
in a demo, or as a lightweight internal tool — without standing up Elasticsearch/Sourcegraph
or cloning everything into a single monolithic index. muck is a small Rust binary
you can `docker run` and start pushing files into within seconds.

## Components

- `src/` — the core server (Rust/axum). In-memory trigram index and own line matcher (no
  vendored search library); pushed file content lives in `mmap`-backed on-disk shard files
  (`src/shard.rs`), not the Rust heap — see [Storage](#storage).
- `src/bin/local.rs` + `ui/` — `muck-local`, a build of the same server with an
  embedded React SPA (Pierre-based diff/tree viewer) for browsing and searching repos in
  a browser. Built behind the `embed-ui` Cargo feature.
- `scripts/index-github-repo.sh` — clones a repo (GitHub URL, `org/repo` shorthand, or an
  Azure DevOps clone URL) and pushes its files into a running muck instance.

## Running locally

```sh
# Plain server (no UI), matches what's deployed
docker build -t muck:local .
docker run -d --name muck -p 7777:7777 muck:local

# With the embedded search UI
docker build -f Dockerfile.local -t muck-local:local .
docker run -d --name muck -p 7777:7777 muck-local:local

# With the embedded UI and a persisted store on a named volume
docker volume create muck-data
docker run -d --name muck -p 7777:7777 \
  -v muck-data:/data \
  muck-local:local

# Or with Docker Compose
# docker compose up --build -d
```

Confirm it's up:

```sh
curl -s http://localhost:7777/health
```

Index a repo:

```sh
./scripts/index-github-repo.sh https://github.com/BurntSushi/ripgrep
```

Then open `http://localhost:7777` (local/UI build) or query directly:

```sh
curl -s -X POST http://localhost:7777/v1/search \
  -H 'Content-Type: application/json' \
  -d '{"query":"TODO"}'
```

By default muck writes pushed file content into shard files under a `muck-shards` directory
in the container's temp dir, and no named volume, so those shards (and the search index) are
gone when the container is removed — same practical effect as "in-memory" for a throwaway
container, just backed by ephemeral container-local disk instead of the heap. See
[Storage](#storage) for why, and set `MUCK_PERSIST_PATH` (see [Persistence](#persistence)) if
you want a restart to not lose everything.

### Filters

`POST /v1/search` accepts a `filters` object alongside `query`/`regex`:

```json
{
  "query": "TODO",
  "filters": {
    "repoIds": ["ripgrep"],
    "fileTypes": ["rs"],
    "pathPrefix": "src/",
    "orgs": ["BurntSushi"],
    "branches": ["main"],
    "globs": ["*.rs", "!*_test.rs"]
  }
}
```

`globs` is ripgrep-style include/exclude glob filtering (`-g`): a plain glob (`*.rs`)
includes matching paths at any depth, a `!`-prefixed glob (`!*_test.rs`) excludes them. An
invalid glob returns `422`.

## Storage

Pushed file content is stored in `mmap`-backed on-disk shard files (one per repo, written
wholesale on every `build` call), not fully resident in the Rust heap. This matters for large
corpora: a prior version of muck kept every pushed file's raw bytes in a `HashMap` for the
life of the process, which meant resident memory grew roughly linearly with total corpus size
and never came back down. With `mmap`, the OS page cache decides what's actually resident —
it shrinks under memory pressure and re-faults cold files back in from disk on the next query
that touches them, without muck doing anything special.

The trigram index itself stays plain heap memory (searching it is on every query's hot path,
so it isn't a disk/mmap candidate the way file content is), but uses a compact flat-buffer
encoding — a single delta+varint-encoded posting buffer plus a lean offset table, not a naive
`HashMap<[u8;3], Vec<u32>>`. That distinction is not cosmetic: a real corpus has millions of
distinct trigrams (rails/rails, ~5,000 files: 3.47 million), and the naive form needs a
separate heap allocation per trigram — millions of tiny allocations, most holding only a
handful of bytes of real data but each still costing a full allocator chunk. See
`src/trigram.rs`'s doc comment and `HANDOFF_STORAGE_OPTIMIZATION.md` for the measured
before/after (resident memory on that same corpus: ~370MB for the naive form, ~64MB for the
compact one).

- Shard files live under `MUCK_SHARD_DIR` if set, otherwise a `muck-shards` subdirectory of
  the OS temp dir. **This means muck always needs local disk for file content** — a fully
  diskless environment isn't supported for that reason; the trigram index (search itself)
  still needs no disk.
- Shards are local-filesystem, single-instance state, same as the persistence backup below —
  give each horizontally-scaled instance its own shard directory.
- See `bench/` for the benchmark suite that measures this in practice (resident memory vs.
  Zoekt/ripgrep/ag across repo sizes).

## Persistence

Set `MUCK_PERSIST_PATH` to a file path to also survive a full process restart (not just the
shard files, which already survive within a single running container unless its filesystem
is itself ephemeral — see [Storage](#storage)):

```sh
docker run -d --name muck -p 7777:7777 \
  -e MUCK_PERSIST_PATH=/data/muck-store.bin \
  -v muck-data:/data \
  muck:local
```

For the embedded UI build, the same setting works:

```sh
docker run -d --name muck -p 7777:7777 \
  -e MUCK_PERSIST_PATH=/data/muck-store.bin \
  -v muck-data:/data \
  muck-local:local
```

- After every `build`/`unregister` call, the full store (every repo's files + metadata) is
  written to that path. The trigram index itself isn't persisted — it's cheap to rebuild
  from the files on load, and skipping it keeps the backup file smaller.
- On startup, if a file already exists at that path, it's loaded in the background and
  `GET /health` returns `503` (`"status":"loading"`) until that load finishes — so a load
  balancer/orchestrator won't route search traffic to a half-populated instance. `200` once
  ready, always, when persistence is disabled.
- **Single instance, local filesystem only.** This is a local backup file, not a shared
  store — there's no locking or multi-writer coordination, and it is not designed to be
  shared across horizontally-scaled instances. If you scale out, give each instance its own
  path/volume (or leave persistence off and re-index on restart). A shared store across
  instances would be a different, separately-designed feature.

## Known limitations

muck's on-disk shard store (see [Storage](#storage)) needs local disk for pushed file
content — there's no fully diskless mode. See
[HANDOFF_STORAGE_OPTIMIZATION.md](HANDOFF_STORAGE_OPTIMIZATION.md) for the design rationale,
the benchmark evidence that motivated it, and why an earlier "drop content after indexing"
approach doesn't work given how search is implemented (`scan_repo` needs full text at query
time; the trigram index is only a candidate filter, not a positional index).

## Development

- Rust server: `cargo build`, `cargo test` from the repo root.
- UI: `cd ui && npm install && npm run dev` (or `npm run build` to produce the embeddable
  `ui/dist` that `Dockerfile.local` bakes into `muck-local`).
- Fuzz targets: see [`fuzz/README.md`](fuzz/README.md).

## Credits

This project started as a fork/port of [momokun7/xgrep](https://github.com/momokun7/xgrep) —
credit to the original author for the core idea and design. Specifically ported so far:

- `src/globfilter.rs` — ported near-verbatim from `rust/src/globfilter.rs` in the original
  project (adapted error type only; logic unchanged).
- `fuzz/` — the cargo-fuzz setup is modeled after the original's `rust/fuzz/`, retargeted at
  this project's own code (see [`fuzz/README.md`](fuzz/README.md) for why the actual fuzz
  targets differ).

Deliberately not carried over (out of scope for a push-based, in-memory server — these are
local-filesystem/CLI concerns better left to whatever indexes and pushes files to
muck): file discovery/`--find`, git-awareness (`--changed`, `--since`,
`.gitignore`), on-disk persistent/incremental indexing, the MCP server, and CLI packaging.
