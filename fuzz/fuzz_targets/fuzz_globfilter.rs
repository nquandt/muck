#![no_main]
use libfuzzer_sys::fuzz_target;
use xgrep_server::globfilter::GlobFilter;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    let globs: Vec<String> = text.lines().map(str::to_string).collect();
    if globs.is_empty() {
        return;
    }
    // Arbitrary glob syntax must never panic — either compiles or returns Err.
    if let Ok(filter) = GlobFilter::new(&globs) {
        // matches() must never panic against an arbitrary candidate path either.
        let _ = filter.matches(text);
    }
});
