# Handoff: reducing muck's resident memory footprint

## TL;DR

muck currently holds every pushed file's full raw bytes in the process heap for as long as it
runs, with no way to reduce that. A real benchmark against rails/rails (4,970 files) showed muck
at **563MB resident memory** vs. Zoekt's index taking **68MB on disk** (Zoekt's *process* memory
in that same run was near-zero, but that's an artifact of how it was benchmarked — see caveats
below, don't take that specific number as "Zoekt uses no memory"). The task: adopt Zoekt's
storage model (on-disk index shards + `mmap`, OS page cache handles hot/cold residency) instead
of muck's current `HashMap<String, Bytes>`, without changing muck's API or losing what makes it
useful (see "Why not just use Zoekt" below — this isn't optional context, read it first).

This was scoped as task #6 in a prior session and never implemented — this doc is everything the
next agent needs to pick it up cold.

## Why this matters (and why it's not "just switch to Zoekt")

The user asked, after seeing these benchmark numbers, whether it even makes sense to keep
maintaining muck instead of just using Zoekt. The answer that was given and should still hold:
Zoekt solves indexing/search, not muck's actual product surface — muck's reason to exist is a
push-arbitrary-files-from-anywhere HTTP API (no git dependency, works for GitHub/ADO/local disk
alike, see `README.md`'s "Why" section) plus a custom UI/UX (search+browse split, link-off
buttons — see `ui/src/routes/`) that would need building on top of Zoekt regardless. The memory
gap is a fixable architecture choice, not a reason to abandon the project. This doc is that fix.

## What NOT to redo: the content-drop dead end

An earlier attempt at this same goal was a `MUCK_CONTENT_MODE=full|context|none` env var meant to
let raw file bytes be dropped after indexing. **This doesn't work and shouldn't be resurrected as
originally scoped.** The reason: `src/search.rs`'s `scan_repo` does a real line-by-line text scan
over each candidate file's raw bytes *at query time* — the trigram index (`src/trigram.rs`) is
only a candidate filter (which files might contain a trigram), not a positional index. It cannot
tell you *where* a match is, only that a file is worth scanning. So there is no way to answer a
search without full text available somewhere, in every mode — dropping it doesn't just lose
snippets, it breaks search entirely. This is why the mmap-shard approach below is the right shape
instead: keep the data available, just not fully resident in the Rust heap all the time.

## The proposed architecture

Modeled on Zoekt/Sourcegraph/Livegrep-style codesearch engines:

- Pushed file content, instead of living in a `HashMap<String, Bytes>` in the Rust heap for the
  lifetime of the process, gets written into a compact **on-disk shard file** per repo (one
  contiguous blob of all the repo's file bytes, with an offset/length table for each path).
- The shard is opened via `mmap` (the `memmap2` crate — not currently a dependency, needs
  adding to `Cargo.toml`) rather than read fully into an owned buffer.
- The trigram postings (`TrigramIndex`) stay RAM-resident as they are today — they're small
  (a few bytes per trigram per file) and there's no reason to change that.
- Actual "resident memory" then becomes whatever the OS's page cache decides to keep warm,
  which shrinks under memory pressure and re-faults from disk on the next query that touches a
  cold file — the "swap"/graceful-degradation behavior the user was originally asking about,
  gotten for free from the kernel instead of hand-built.

**This still needs local disk.** It does not help a fully diskless sandbox — that case remains
either RAM-only (today's behavior) or "search elsewhere" (the link-off feature). That's an
accepted, documented limitation, not a gap to solve here.

## Code map — what actually has to change

- **`src/store.rs`** — `RepoData.files: HashMap<String, Bytes>` is the thing being replaced.
  - `put_file` (`src/handlers.rs` calls this from `PUT /v1/repos/{repoId}/files`) currently
    inserts into that map one file at a time, before a `build` call. Files pushed but not yet
    built have to go somewhere — simplest option: keep the current in-heap buffering during the
    push phase (it's transient, bounded by one push burst, and already the existing behavior),
    and only write to the mmap'd shard during `build()`. Alternative (more work, not necessary
    for a first cut): stream pushed files straight to a growing shard file as they arrive. Pick
    based on how bad push-phase memory actually turns out to be in practice — start with the
    simpler option.
  - `build()` (called from `POST /v1/repos/{repoId}/build`) is where the shard gets written:
    serialize the repo's current file set into a single shard file on disk, replace
    `RepoData.files` with something like `file_offsets: HashMap<String, (u64, u32)>` plus a
    shared `memmap2::Mmap` handle, and free the transient push-phase bytes. Comment in the
    existing code already documents that the index is "rebuilt wholesale on each build call" —
    rewriting the shard wholesale on every build is consistent with that, not a new constraint.
  - `get_file`, `list_paths`, `delete_file`, `unregister` all touch `repo.files` today and need
    the equivalent shard+offset-based access path.
- **`src/trigram.rs`** — good news: `TrigramIndex::build(docs: &[impl AsRef<[u8]>])` is already
  generic over anything that derefs to `&[u8]`, not tied to `Bytes` specifically. A slice into an
  mmap'd region satisfies `AsRef<[u8]>` fine — **this file likely needs zero changes.**
- **`src/search.rs`** — `RepoSnapshot.candidates: Vec<(String, Bytes)>` and `snapshot_candidates`
  currently clone `Bytes` (cheap, refcounted) out of the repo's file map under a read lock, then
  `scan_repo` does the line-by-line text scan over those bytes off the async runtime. This needs
  to become slices into the mmap'd shard instead of `Bytes` clones — the shape of the code
  (snapshot under a lock, scan off-thread) doesn't need to change, just what it's holding a
  reference to.
- **`src/persist.rs`** — currently bincode-serializes every file's full bytes into a backup file
  behind `MUCK_PERSIST_PATH`. Once file content lives in an on-disk shard already, this could
  simplify significantly (the shard *is* the persisted content — persistence might become "keep
  the shard file across restarts" rather than a separate bincode blob of the same bytes). Worth
  reconsidering as part of this work, not strictly required for the memory-footprint goal itself.
- **`Cargo.toml`** — add `memmap2` (or equivalent) as a dependency.

## Suggested step-by-step plan

1. Add `memmap2`, design the shard file format (a simple length-prefixed concatenation of file
   bytes + a separate offset table is enough — don't over-engineer this, Zoekt's actual shard
   format has features like compression/symbol-search that aren't needed here).
2. Change `RepoData` in `store.rs` to hold shard offsets + an `Arc<memmap2::Mmap>` instead of
   `HashMap<String, Bytes>`; update `build()` to write the shard and swap it in.
3. Update `get_file`/`list_paths`/`delete_file`/`unregister` to use the new representation.
4. Update `search.rs`'s `RepoSnapshot`/`snapshot_candidates`/`scan_repo` to read from mmap'd
   slices instead of `Bytes` clones.
5. Run the existing test suite (`cargo test`) — there are unit tests in `trigram.rs` and
   presumably integration coverage elsewhere; make sure nothing in `search`/`store` regresses.
6. **Verify with the benchmark suite that already exists** (`bench/`) — this is the concrete,
   already-built way to prove this worked:
   ```bash
   ./bench/run.sh --tier medium --repo rails --tools muck
   ```
   Compare the "Warm-state memory" number in the generated summary against the baseline already
   on record: **563.3MB** resident for muck on rails/rails (4,970 files), captured
   2026-07-23. A successful fix should show a large drop in that number after an idle period
   (nothing has queried the corpus yet, so nothing's been faulted back into the page cache) —
   confirm this against a synthetic memory-pressure scenario too if possible, not just the
   idle-after-build snapshot the harness currently takes.
7. Update `README.md`'s persistence section and `src/store.rs`'s doc comments — several of them
   currently assert things like "files is the only copy of the content that exists anywhere in
   this process" (`store.rs`) which will no longer be true and will actively mislead the next
   reader if left unchanged.

## Open decisions for whoever picks this up

- **Push-phase buffering**: in-heap until `build()`, or stream straight to shard? (See above —
  start simple, only complicate if it's actually a problem.)
- **Shard format**: custom minimal format vs. reusing something existing? A custom format is
  almost certainly less work than adapting Zoekt's actual shard format, which carries features
  (symbol search, compression, compound shards) muck doesn't need.
- **Persistence overlap**: does `MUCK_PERSIST_PATH` get simplified/merged with the shard file, or
  stay a separate concern? Not required to resolve this issue, but touches the same code and is
  worth deciding deliberately rather than accidentally.
- **Windows dev support**: `mmap` behaves slightly differently on Windows vs Unix (file locking
  semantics especially) — this repo is developed partly on Windows (see `bench/ZOEKT_SETUP.md`
  and various Windows/git-bash notes in `bench/lib.sh` for how much that's already bitten this
  project once). Test `cargo build`/`cargo test` on Windows before assuming Unix-only mmap
  behavior is fine, or scope this to Linux-only if that's an acceptable tradeoff.

## Where the supporting evidence lives

- `bench/` — the full benchmark suite (muck vs. Zoekt vs. ripgrep vs. ag), including the
  memory/disk measurement this doc's baseline number came from. `bench/README.md` explains the
  methodology and caveats (including why Zoekt's process-memory number in these benchmarks isn't
  directly comparable — it's only ever run as one-shot CLI calls in the harness, not a live
  `zoekt-webserver`, so its "resident memory" reading is an idle-shell artifact, not what a real
  Zoekt deployment would show).
- `bench/results/` — raw run output is gitignored, so the 563.3MB number above is only recorded
  in this doc and in prior conversation, not as a committed artifact. Re-run the suite to
  reproduce it before relying on it as precise, but it should be directionally consistent.
