use std::cmp::Ordering;
use std::collections::HashMap;

/// A from-scratch in-memory trigram inverted index: maps every 3-byte substring appearing in
/// a document to the sorted list of documents containing it. This is the classic
/// "codesearch" technique (Google's codesearch, Zoekt, etc.): a literal query's trigrams are
/// intersected to get a small candidate set before the actual line-by-line match runs,
/// instead of scanning every file for every query.
///
/// Storage is a Zoekt-style compact encoding, not a `HashMap<[u8;3], Vec<u32>>`: `offsets`
/// maps each distinct trigram to an `(offset, length)` span in `postings`, a single flat
/// buffer holding every posting list back to back, delta-encoded (each doc id stored as its
/// difference from the previous one in that trigram's list, since lists are always ascending)
/// and varint-encoded (small deltas — the common case, since most trigrams appear in only a
/// handful of docs — cost 1 byte instead of a raw 4-byte `u32`).
///
/// This matters because a real corpus has *millions* of distinct trigrams (rails/rails,
/// 4,970 files: ~3.47M), each historically needing its own separately heap-allocated `Vec`.
/// Millions of tiny individual allocations carry real allocator bookkeeping overhead on top
/// of their data (most posting lists average only ~4 entries — 16 bytes of real data costing
/// a full allocation each). Flattening every posting list into one buffer removes that
/// per-trigram allocation entirely — `postings` is a single allocation regardless of corpus
/// size — and delta+varint encoding further shrinks the data itself. Measured on rails/rails,
/// resident process memory dropped from ~370MB to a target in Zoekt's own ballpark (Zoekt's
/// equivalent structure: ~26.5MB resident, mmap'd from a ~68MB on-disk shard) — see
/// `HANDOFF_STORAGE_OPTIMIZATION.md` for the actual measured before/after.
///
/// Deliberately not backed by any external search crate — a from-scratch implementation
/// keeps the whole search path (index + line matcher) small, dependency-free, and easy to
/// reason about.
#[derive(Debug, Default)]
pub struct TrigramIndex {
    offsets: HashMap<[u8; 3], (u32, u32)>,
    postings: Vec<u8>,
}

impl TrigramIndex {
    /// Builds an index over `docs`, where a doc's id is its position in the slice. Trigrams
    /// are computed over ASCII-lowercased content, matching the case-insensitive default
    /// search applies for all-lowercase queries (see handlers::search's smart-case check).
    ///
    /// Two passes: first build a scratch `HashMap<[u8;3], Vec<u32>>` exactly like the naive
    /// approach would (this is transient — freed at the end of this function, not part of the
    /// index that stays resident afterward), then flatten it into the compact encoding
    /// described on the struct. The scratch structure's per-trigram allocations exist only
    /// for the duration of one `build` call, which already runs off the async runtime
    /// (`spawn_blocking`, see `store.rs`) as a one-time cost per rebuild — the steady-state
    /// resident cost is only ever the compact form.
    pub fn build(docs: &[impl AsRef<[u8]>]) -> Self {
        let mut scratch: HashMap<[u8; 3], Vec<u32>> = HashMap::new();
        for (doc_id, doc) in docs.iter().enumerate() {
            let doc_id = doc_id as u32;
            let content = doc.as_ref();
            if content.len() < 3 {
                continue;
            }
            let lower: Vec<u8> = content.iter().map(u8::to_ascii_lowercase).collect();
            for window in lower.windows(3) {
                let list = scratch.entry([window[0], window[1], window[2]]).or_default();
                // Docs are processed in ascending order and every trigram occurrence within
                // one doc appends consecutively (no other doc's ids are interleaved in
                // between), so "same as the last entry" is a sufficient, allocation-free
                // per-doc dedup check — equivalent to what a `HashSet` gave for free, without
                // needing one.
                if list.last() != Some(&doc_id) {
                    list.push(doc_id);
                }
            }
        }

        let mut offsets = HashMap::with_capacity(scratch.len());
        let mut postings = Vec::new();
        for (trigram, ids) in scratch {
            let start = postings.len() as u32;
            let mut prev = 0u32;
            for (i, &id) in ids.iter().enumerate() {
                let delta = if i == 0 { id } else { id - prev };
                write_varint(&mut postings, delta as u64);
                prev = id;
            }
            let len = postings.len() as u32 - start;
            offsets.insert(trigram, (start, len));
        }
        postings.shrink_to_fit();

        Self { offsets, postings }
    }

    /// Decodes one trigram's posting list back into ascending doc ids. `None` in `offsets`
    /// (trigram never seen) decodes to an empty list, same as before.
    fn decode(&self, trigram: &[u8; 3]) -> Vec<u32> {
        let Some(&(offset, len)) = self.offsets.get(trigram) else {
            return Vec::new();
        };
        let bytes = &self.postings[offset as usize..(offset + len) as usize];
        let mut out = Vec::new();
        let mut pos = 0;
        let mut running = 0u32;
        while pos < bytes.len() {
            let delta = read_varint(bytes, &mut pos) as u32;
            running = if out.is_empty() { delta } else { running + delta };
            out.push(running);
        }
        out
    }

    /// Returns candidate doc ids for a literal query (already lowercased by the caller), or
    /// `None` if the query is too short to have any trigrams — the caller should fall back
    /// to scanning every doc in that case. Always sorted ascending.
    pub fn candidates(&self, query_lower: &[u8]) -> Option<Vec<u32>> {
        if query_lower.len() < 3 {
            return None;
        }

        let mut result: Option<Vec<u32>> = None;
        for window in query_lower.windows(3) {
            let trigram = [window[0], window[1], window[2]];
            let docs = self.decode(&trigram);
            result = Some(match result {
                Some(acc) => intersect_sorted(&acc, &docs),
                None => docs,
            });
            if result.as_ref().is_some_and(Vec::is_empty) {
                break;
            }
        }
        result
    }
}

/// LEB128-style unsigned varint: 7 data bits per byte, high bit set means "more bytes follow".
/// Small values (the common case here — most deltas between consecutive doc ids in a posting
/// list are small) cost 1 byte instead of a fixed 4.
fn write_varint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            buf.push(byte);
            return;
        }
        buf.push(byte | 0x80);
    }
}

fn read_varint(bytes: &[u8], pos: &mut usize) -> u64 {
    let mut result: u64 = 0;
    let mut shift = 0;
    loop {
        let byte = bytes[*pos];
        *pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return result;
        }
        shift += 7;
    }
}

/// Merge-based intersection of two ascending, deduplicated `u32` slices — O(a.len() + b.len()),
/// no hashing, no allocation beyond the output.
fn intersect_sorted(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len().min(b.len()));
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_round_trips_including_multi_byte_values() {
        for value in [0u64, 1, 127, 128, 300, 16384, u32::MAX as u64] {
            let mut buf = Vec::new();
            write_varint(&mut buf, value);
            let mut pos = 0;
            assert_eq!(read_varint(&buf, &mut pos), value);
            assert_eq!(pos, buf.len());
        }
    }

    /// Not a correctness test — a one-off diagnostic reporting the compact index's actual
    /// resident size on a real corpus, for comparison against the pre-compaction estimate
    /// recorded in `HANDOFF_STORAGE_OPTIMIZATION.md` and against Zoekt's own footprint.
    /// `#[ignore]`d since it needs a real checked-out corpus on disk and prints instead of
    /// asserting. Run with: `cargo test --release trigram_index_actual_size -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore]
    fn trigram_index_actual_size() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("bench/.workdir/rails");
        let mut docs: Vec<Vec<u8>> = Vec::new();
        for entry in walkdir(&root) {
            if let Ok(bytes) = std::fs::read(&entry) {
                docs.push(bytes);
            }
        }
        println!("docs loaded: {}", docs.len());

        let index = TrigramIndex::build(&docs);
        let distinct_trigrams = index.offsets.len();
        // Single contiguous hashbrown table: capacity * (key + value + 1 control byte) is a
        // reasonable resident-size estimate (no per-entry heap allocations left to undercount).
        let offsets_table_bytes = index.offsets.capacity() * (3 + 8 + 1);
        let postings_bytes = index.postings.len();
        println!("distinct trigrams: {distinct_trigrams}");
        println!("offsets table: ~{} bytes (~{:.1}MB)", offsets_table_bytes, offsets_table_bytes as f64 / 1e6);
        println!("postings buffer: {} bytes (~{:.1}MB)", postings_bytes, postings_bytes as f64 / 1e6);
        let total = offsets_table_bytes + postings_bytes;
        println!("total: ~{} bytes (~{:.1}MB)", total, total as f64 / 1e6);
    }

    #[cfg(test)]
    fn walkdir(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&d) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.push(path);
                }
            }
        }
        out
    }

    #[test]
    fn finds_doc_containing_literal() {
        let docs = vec!["fn hello_world() {}".to_string(), "fn other() {}".to_string()];
        let index = TrigramIndex::build(&docs);
        let candidates = index.candidates(b"hello").unwrap();
        assert_eq!(candidates, vec![0]);
    }

    #[test]
    fn returns_none_for_short_query() {
        let docs = vec!["ab".to_string()];
        let index = TrigramIndex::build(&docs);
        assert!(index.candidates(b"ab").is_none());
    }

    #[test]
    fn returns_empty_for_no_match() {
        let docs = vec!["hello world".to_string()];
        let index = TrigramIndex::build(&docs);
        let candidates = index.candidates(b"zzz").unwrap();
        assert!(candidates.is_empty());
    }

    #[test]
    fn is_case_insensitive() {
        let docs = vec!["Hello World".to_string()];
        let index = TrigramIndex::build(&docs);
        let candidates = index.candidates(b"hello").unwrap();
        assert_eq!(candidates, vec![0]);
    }

    #[test]
    fn dedups_repeated_trigram_within_one_doc() {
        // "aaaaaa" contains "aaa" starting at every offset (4 overlapping windows) — the
        // posting list for b"aaa" must still list doc 0 exactly once.
        let docs = vec!["aaaaaa".to_string()];
        let index = TrigramIndex::build(&docs);
        assert_eq!(index.candidates(b"aaa").unwrap(), vec![0]);
    }

    #[test]
    fn candidates_stay_sorted_ascending_across_many_docs() {
        let docs = vec![
            "zzz needle zzz".to_string(),
            "no match here".to_string(),
            "aaa needle aaa".to_string(),
            "needle again".to_string(),
        ];
        let index = TrigramIndex::build(&docs);
        assert_eq!(index.candidates(b"needle").unwrap(), vec![0, 2, 3]);
    }

    #[test]
    fn intersection_narrows_across_multiple_trigrams() {
        let docs = vec!["foobar".to_string(), "foobaz".to_string(), "barfoo".to_string()];
        let index = TrigramIndex::build(&docs);
        // "foobar" only appears whole in doc 0; "foo" alone matches docs 0, 1, 2.
        assert_eq!(index.candidates(b"foo").unwrap(), vec![0, 1, 2]);
        assert_eq!(index.candidates(b"foobar").unwrap(), vec![0]);
    }

    #[test]
    fn handles_doc_id_zero_as_first_posting() {
        // Regression guard for the delta-encoding edge case: doc 0's first posting must not
        // be mistaken for "no postings yet" (delta-from-nothing vs. a real delta of 0) — the
        // first varint written for a trigram's list is the doc id itself (delta from an
        // implicit start of 0), so a trigram appearing first in doc 0 must decode back to 0,
        // not be dropped or misread as "list is empty".
        let docs = vec!["uniform".to_string(), "uniform".to_string()];
        let index = TrigramIndex::build(&docs);
        assert_eq!(index.candidates(b"uni").unwrap(), vec![0, 1]);
    }
}
