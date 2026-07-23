use std::collections::{HashMap, HashSet};

/// A from-scratch in-memory trigram inverted index: maps every 3-byte substring appearing in
/// a document to the set of documents containing it. This is the classic "codesearch"
/// technique (Google's codesearch, Zoekt, etc.): a literal query's trigrams are intersected
/// to get a small candidate set before the actual line-by-line match runs, instead of
/// scanning every file for every query.
///
/// Deliberately not backed by any external search crate — see
/// openspec/specs/ado-code-search/HANDOFF.md for why xgrep-server owns this outright.
#[derive(Debug, Default)]
pub struct TrigramIndex {
    postings: HashMap<[u8; 3], HashSet<u32>>,
}

impl TrigramIndex {
    /// Builds an index over `docs`, where a doc's id is its position in the slice. Trigrams
    /// are computed over ASCII-lowercased content, matching the case-insensitive default
    /// search applies for all-lowercase queries (see handlers::search's smart-case check).
    pub fn build(docs: &[impl AsRef<[u8]>]) -> Self {
        let mut postings: HashMap<[u8; 3], HashSet<u32>> = HashMap::new();
        for (doc_id, doc) in docs.iter().enumerate() {
            let content = doc.as_ref();
            if content.len() < 3 {
                continue;
            }
            let lower: Vec<u8> = content.iter().map(u8::to_ascii_lowercase).collect();
            for window in lower.windows(3) {
                postings
                    .entry([window[0], window[1], window[2]])
                    .or_default()
                    .insert(doc_id as u32);
            }
        }
        Self { postings }
    }

    /// Returns candidate doc ids for a literal query (already lowercased by the caller), or
    /// `None` if the query is too short to have any trigrams — the caller should fall back
    /// to scanning every doc in that case. Sorted ascending: `HashSet`'s iteration order is
    /// randomized per-instance (a fresh random hasher seed on every intersection), so leaving
    /// it unsorted made identical repeated queries come back in a different order each time.
    pub fn candidates(&self, query_lower: &[u8]) -> Option<Vec<u32>> {
        if query_lower.len() < 3 {
            return None;
        }

        let mut result: Option<HashSet<u32>> = None;
        for window in query_lower.windows(3) {
            let trigram = [window[0], window[1], window[2]];
            let docs = self.postings.get(&trigram).cloned().unwrap_or_default();
            result = Some(match result {
                Some(acc) => acc.intersection(&docs).copied().collect(),
                None => docs,
            });
            if result.as_ref().is_some_and(HashSet::is_empty) {
                break;
            }
        }
        result.map(|set| {
            let mut ids: Vec<u32> = set.into_iter().collect();
            ids.sort_unstable();
            ids
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
