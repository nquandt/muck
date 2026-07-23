#![no_main]
use libfuzzer_sys::fuzz_target;
use xgrep_server::trigram::TrigramIndex;

fuzz_target!(|data: &[u8]| {
    // Split arbitrary input into documents on newlines; the last chunk doubles as the query.
    let mut parts: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();
    let query = parts.pop().unwrap_or(&[]);
    let docs: Vec<&[u8]> = parts;

    let index = TrigramIndex::build(&docs);
    if let Some(candidates) = index.candidates(query) {
        // Candidate ids: valid doc indices, strictly ascending (sorted, deduped).
        assert!(candidates.windows(2).all(|w| w[0] < w[1]));
        assert!(candidates.iter().all(|&id| (id as usize) < docs.len()));
    }
});
