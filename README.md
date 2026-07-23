# muck

A fast, in-memory code search server with an embedded GitHub-code-search-style UI. Push
files from any repo (GitHub, Azure DevOps, or a local checkout) into an in-memory trigram
index and search across them over HTTP — no disk, no persistent state, no external
dependencies by default. An optional local-disk backup can be enabled to survive restarts
(see [Persistence](#persistence)).

## Why

Sometimes you just want fast full-text/regex search across a pile of repos — locally,
in a demo, or as a lightweight internal tool — without standing up Elasticsearch/Sourcegraph
or cloning everything into a single monolithic index. muck is a small Rust binary
you can `docker run` and start pushing files into within seconds.

## Components

- `src/` — the core server (Rust/axum). Purely in-memory trigram index, own line matcher,
  no vendored search library.
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

muck is purely in-memory — no volumes, no config. Restart the container to reset it.

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

## Persistence

muck is in-memory by default — a restart loses everything, and you'd need to
re-push and rebuild every repo. Set `MUCK_PERSIST_PATH` to a file path to enable an
optional local-disk backup instead:

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
