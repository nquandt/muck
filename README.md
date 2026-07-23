# xgrep-server

A fast, in-memory code search server with an embedded GitHub-code-search-style UI. Push
files from any repo (GitHub, Azure DevOps, or a local checkout) into an in-memory trigram
index and search across them over HTTP — no disk, no persistent state, no external
dependencies.

## Why

Sometimes you just want fast full-text/regex search across a pile of repos — locally,
in a demo, or as a lightweight internal tool — without standing up Elasticsearch/Sourcegraph
or cloning everything into a single monolithic index. xgrep-server is a small Rust binary
you can `docker run` and start pushing files into within seconds.

## Components

- `src/` — the core server (Rust/axum). Purely in-memory trigram index, own line matcher,
  no vendored search library.
- `src/bin/local.rs` + `ui/` — `xgrep-server-local`, a build of the same server with an
  embedded React SPA (Pierre-based diff/tree viewer) for browsing and searching repos in
  a browser. Built behind the `embed-ui` Cargo feature.
- `scripts/index-github-repo.sh` — clones a repo (GitHub URL, `org/repo` shorthand, or an
  Azure DevOps clone URL) and pushes its files into a running xgrep-server instance.

## Running locally

```sh
# Plain server (no UI), matches what's deployed
docker build -t xgrep-server:local .
docker run -d --name xgrep-server -p 7777:7777 xgrep-server:local

# With the embedded search UI
docker build -f Dockerfile.local -t xgrep-server-local:local .
docker run -d --name xgrep-server -p 7777:7777 xgrep-server-local:local
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

xgrep-server is purely in-memory — no volumes, no config. Restart the container to reset it.

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

## Development

- Rust server: `cargo build`, `cargo test` from the repo root.
- UI: `cd ui && npm install && npm run dev` (or `npm run build` to produce the embeddable
  `ui/dist` that `Dockerfile.local` bakes into `xgrep-server-local`).
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
xgrep-server): file discovery/`--find`, git-awareness (`--changed`, `--since`,
`.gitignore`), on-disk persistent/incremental indexing, the MCP server, and CLI packaging.
