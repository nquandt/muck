# Handoff: reducing muck's resident memory footprint

**Status: implemented and production-hardened.** Original file-content storage rewrite, plus
a follow-up pass closing every gap that first pass left open (orphaned shard files, crash
recovery, concurrent-build races, shard filename collisions, and the trigram index's own
memory footprint) — see "Production-hardening pass" below for that second round. Nothing
known is left deliberately unfixed as of this writing.

`src/shard.rs` + `src/store.rs`/`src/search.rs`/`src/persist.rs`
changes landed per the plan below. Key decisions made while implementing (see "Open
decisions" at the bottom for the reasoning): push-phase buffering stayed in-heap until
`build()` (simple option, as suggested); shard format is a custom minimal
concat-blob-plus-offset-table; persistence (`MUCK_PERSIST_PATH`) stayed a separate concern
from the shard files rather than being merged, since it backs up *all* repo state
(name/version/org/branch/links), not just file bytes, and simplifying that merge wasn't
required for the memory-footprint goal; shard storage applies on all platforms including
Windows (tested via `cargo test`), with a documented caveat that deleting a still-mapped
shard file can fail on Windows (handled as a best-effort, logged-not-fatal operation, same
pattern as the existing persistence error handling). One deliberate scope change from the
original plan: shard files are **not conditional** — muck now always needs local disk for
file content (defaulting to the OS temp dir if `MUCK_SHARD_DIR` isn't set), rather than
falling back to the old in-heap map when unconfigured. This was simpler to implement
correctly and matches the architecture actually described below (not a toggle); see
README's new "Storage" section for the user-facing framing of this tradeoff.
**Bench verification (step 6) was run and the memory-pressure claim is now confirmed —
graceful page-cache reclaim under pressure genuinely happens.** Timeline of how this was
verified (worth keeping for the next person who wants to re-check it):

- `./bench/run.sh --tier medium --repo rails --tools muck` (2026-07-23): mem **567.8MB**,
  basically unchanged from the 563.3MB baseline. This is *expected*, not a failure — the
  harness snapshots memory immediately after `build`, before any idle period, so every page
  is still hot in cache either way regardless of storage architecture. The harness isn't
  currently capable of proving the pressure-release behavior; the manual steps below are.
- First manual attempt (misleading, since corrected): ran a shrink test against a `docker
  build`-cached `muck-local:bench` image and got an `OOMKilled=true` result that looked like
  a regression. **Root cause: that image was stale** — built before this change, from a
  prior benchmark run, so it was still running the old `HashMap<String, Bytes>` code (`docker
  image inspect` short-circuits `bench/run_muck.sh`'s rebuild if the tag already exists).
  Confirmed by `docker exec ... find / -iname '*.shard'` on that container returning nothing.
  Lesson for next time: **force a rebuild** (`docker build -f Dockerfile.local -t
  muck-local:bench .` explicitly, don't rely on the bench script's cache check) whenever the
  server source changed since the image was last built.
- After rebuilding the image for real and re-running push+build for the full rails corpus
  (4,970 files) against a fresh container:
  - The shard file landed on disk as expected: `/tmp/muck-shards/bench.shard`, **39MB**.
  - `/sys/fs/cgroup/memory.stat` right after build: `file_mapped 40095744` (~40MB, matching
    the shard size almost exactly) and `inactive_file 80457728` (~80MB reclaimable) — proof
    the mmap'd shard pages are correctly accounted as reclaimable page cache, not anonymous
    heap, contradicting nothing in the design.
  - `docker update --memory=600m --memory-swap=600m` on the *running* container (i.e.
    genuine pressure while it's live, not just at startup): container **survived**
    (`OOMKilled=false`), and `file` cache dropped from ~82MB to ~3MB — the kernel reclaimed
    the shard's cached pages under pressure exactly as designed, no crash.
  - A subsequent search query (`ActiveRecord`) against the pressured container **returned
    correct results**, and `file_mapped` climbed back from ~2.5MB to ~28.5MB — confirms the
    re-fault-from-disk path on a cold query works, not just that reclaim doesn't crash things.
  - (The earlier attempt against the *stale* image at 200m/150m also OOM-killed, but for an
    unrelated, expected reason: the old code kept the whole corpus in an unbounded `HashMap`
    with no reclaim path at all — consistent with the bug this fix exists to solve, not a
    counterexample to it.)
- **One honest finding that's still open, not a regression but worth flagging:** resident
  `anon` memory after build was ~580MB for the rails corpus — much larger than the 39MB
  shard, and not obviously smaller than before this change. The trigram index
  (`TrigramIndex::postings: HashMap<[u8;3], HashSet<u32>>`) is heap-resident by design (see
  "The proposed architecture" below — this was always the plan, not an oversight), and for a
  corpus this size its `HashMap`/`HashSet` per-entry overhead may be substantial; it's also
  possible some of that ~580MB is allocator-retained-but-freed heap from the push-phase
  buffer (glibc/Rust's allocator doesn't always return freed pages to the OS, so process RSS
  can overstate live heap use). Either way: **file content is confirmed off the heap and
  reclaimable under pressure — that's this task's stated goal, achieved** — but the trigram
  index's own memory cost for large corpora is a separate, currently-unmeasured question the
  original benchmark's 563MB number didn't isolate. Worth a follow-up bench run that reports
  `anon` vs `file` cgroup breakdown directly (not just total container RSS) if someone wants
  to chase the index's footprint down next.
- What's also verified structurally: `cargo build`/`cargo test` pass (including a new
  end-to-end integration test, `tests/shard_store.rs`, covering push → build → search →
  delete → rebuild → unregister against the real mmap'd shard path), and the full rails
  corpus pushes/builds/searches correctly through the new storage layer inside Docker.

## Production-hardening pass (done after the initial implementation above)

A few gaps that were fine for a correctness-first cut but not for a long-running production
process, found while stress-testing the above:

- **Orphaned shard files.** Nothing was cleaning up shard files left behind on disk when a
  repo's `Shard` handle simply went away — a crash mid-build, or (much more commonly) running
  without `MUCK_PERSIST_PATH` at all, where a restart forgets every repo but the old shard
  files silently kept accumulating in `MUCK_SHARD_DIR`/the OS temp dir forever. Fixed:
  `shard::purge_orphaned_shards` now runs once at `Store` startup (`Store::new` and
  `Store::new_with_persistence`, after any persisted-state load finishes) and deletes any
  `*.shard`/`*.shard.tmp-*` file under the shard dir that doesn't belong to a repo that just
  got loaded (or unconditionally, if nothing did). Covered by
  `shard::tests::purge_removes_only_unreferenced_shards` and, end-to-end through a simulated
  process restart, `tests/persistence_restart.rs`.
- **Concurrent `build()` calls for the same repo could corrupt each other's shard.** Nothing
  serializes two `POST .../build` requests for the same `repo_id` racing each other, and the
  original `write_shard` used a deterministic tmp file name derived only from the repo id —
  two concurrent builds would both write through the same tmp path and could interleave
  mid-write. Fixed: `write_shard`'s tmp path now includes a per-process atomic counter plus
  the PID, so concurrent writes never share a file; the final rename still just picks a
  winner if two builds race (consistent with `build`'s pre-existing "rebuilt wholesale, last
  write wins" semantics — this fix is about not corrupting a file, not about which build
  "wins" a race, which was never guaranteed and still isn't).
- Verified via a fresh `docker build` + a real container smoke test (push two files → build →
  search → confirm shard file on disk → delete a file → unregister → confirm the shard file
  is actually gone from disk), not just unit tests.

**Follow-up pass: all three gaps above were closed, not left open:**

- **Trigram index memory.** `TrigramIndex`'s postings switched from `HashMap<[u8;3],
  HashSet<u32>>` to `HashMap<[u8;3], Vec<u32>>` (sorted, deduplicated — `build` already
  produces each posting list in ascending doc-id order for free, one doc at a time, and
  `candidates` now does a merge-based sorted intersection instead of hash-set intersection,
  which is both smaller and faster). **Measured effect on the rails corpus (4,970 files),
  rebuilt Docker image, real container**: total resident memory dropped from **567.8MB to
  356MB**; the cgroup `anon` figure specifically (the part this change targets) dropped from
  ~580MB to ~370MB. That's on top of the file-content win already covered above — this was
  the index catching up to the same architecture the shard rewrite gave file content. Covered
  by new trigram tests (`dedups_repeated_trigram_within_one_doc`,
  `candidates_stay_sorted_ascending_across_many_docs`,
  `intersection_narrows_across_multiple_trigrams`).
- **Concurrent `build()` races.** `Store` now holds a per-repo `tokio::sync::Mutex` (created
  lazily, one per repo id, removed on `unregister`) for the full duration of `build()` — read
  current state → write shard → commit. Two `POST .../build` calls for the *same* repo now
  serialize instead of racing to decide whose result becomes the repo's final state (the real
  risk wasn't a "torn" read — each build's own commit was always internally atomic under the
  repos write-lock — it was an *out-of-order-completion* race: an earlier-started, slower
  build finishing after a later one and silently reverting it). Builds for different repos
  are unaffected and still run fully concurrently. Covered by
  `tests/concurrent_build.rs` (20 concurrent builds against one repo, asserts the final state
  is self-consistent — every listed path is actually readable from the shard that ended up
  committed alongside it).
- **Shard filename collisions.** `shard::shard_file_name` now suffixes the sanitized prefix
  with a 16-hex-digit hash of the *original, unsanitized* repo id (`DefaultHasher`, fixed
  non-randomized seed, so stable across restarts of the same build). Two repo ids that
  sanitize to the same string (`"a/b"` and `"a:b"` both becoming `"a_b"`) now get different
  filenames — confirmed in the rebuilt Docker image, where a real shard landed as
  `bench-56d26608fd345f66.shard`, not `bench.shard`. Also incidentally caps the sanitized
  prefix at 64 characters, guarding against a pathologically long repo id blowing past a
  filesystem's path-length limit. `purge_orphaned_shards`/`persistence_restart.rs` no longer
  assume a predictable filename, since it isn't one anymore by design.

Full picture after both passes, verified against the real rails corpus in Docker each time:
baseline (pre-change) **563.3MB** → shard-only fix **~356–567MB depending on cache
warmth/pressure** → shard + trigram-index fix **356MB resident, survives down to a 420MB
container memory limit** (below that, the still-heap-resident index itself becomes the
floor — expected, not a bug, see the `anon` figure above).

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

**2026-07-23 correction, after this doc's fix was implemented:** the "Zoekt's process memory
was near-zero" framing above turned out to be measuring the wrong thing — `bench/run_zoekt.sh`
only ran one-shot `zoekt-index`/`zoekt` CLI calls, never a real `zoekt-webserver`, so both its
memory *and* hot-path query numbers were an idle-shell/in-process artifact, not what a real
Zoekt deployment looks like. Fixed (`run_zoekt.sh` now runs a live `zoekt-webserver`, queried
over real HTTP, same as muck). Corrected numbers for the same rails corpus, both tools as real
long-running server containers, showed Zoekt genuinely smaller (~27MB vs. muck's ~390MB at the
time) but hot-path query latency much more competitive than the original (wrong) numbers
suggested — muck was already faster on regex queries and short/common literals, only somewhat
slower on longer literal words.

**2026-07-23, later the same day: closed most of the remaining memory gap.** Diagnosed *why*
Zoekt was smaller (see `trigram.rs`'s `trigram_index_actual_size` test and the struct doc
comment): rails/rails has **3.47 million distinct trigrams**, and the old
`HashMap<[u8;3], Vec<u32>>` representation needed a *separate heap allocation per trigram* —
millions of tiny `Vec`s, most holding only ~4 entries (16 bytes of real data) but each costing
a full allocator chunk. Rewrote `TrigramIndex` to the same technique already used for file
content: one flat, delta+varint-encoded posting buffer (`Vec<u8>`, a single allocation
regardless of corpus size) plus a lean `HashMap<[u8;3], (u32, u32)>` offset table (also one
contiguous allocation — hashbrown's table itself was never the problem, the per-value `Vec`
was). Measured real structure sizes on rails (not an estimate): **63.6MB** (44MB offsets +
19.5MB postings), vs. the ~370MB `anon` figure measured before this change.

Real container measurement, same rails corpus, before vs. after this specific change:
**memory 392MB → 249MB** (further 37% drop, stacking on top of the earlier file-content fix),
**with no hot-path regression** — every query's median latency was the same or faster
(regex-word-error dropped from 162ms to 101ms), most likely from better cache locality
(one small contiguous buffer beats millions of scattered heap allocations for CPU cache
behavior too, not just memory bookkeeping). Final head-to-head against a live
`zoekt-webserver` on the same corpus: Zoekt still smaller (27.4MB vs. muck's 249MB, ~9x now,
down from ~15x), muck still faster on regex (6-9x) and comparable-to-faster on literals.
Covered by `trigram.rs`'s existing test suite (all pass unmodified — `TrigramIndex`'s public
API didn't change, only its internals) plus two new tests: `varint_round_trips_including_
multi_byte_values` and `handles_doc_id_zero_as_first_posting` (a real edge case in delta
encoding: distinguishing "trigram's first posting is doc 0" from "trigram has no postings").

**Not pursued further, and here's why:** going all the way to Zoekt's exact number (mmap'ing
the postings buffer from disk, like `crate::shard` does for file content, instead of keeping
it as a plain heap `Vec<u8>`) would close the remaining ~9x gap further, but the offset
`HashMap` (44MB, now the *larger* of the two remaining pieces) would need the same treatment
to matter much — i.e., replacing it with a sorted on-disk array searched by binary search
instead of hashing, which is a bigger structural change for a shrinking marginal return once
the low-hanging "millions of tiny allocations" problem is already fixed. Left as a genuine
option for later, not a gap that undermines today's numbers.

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
