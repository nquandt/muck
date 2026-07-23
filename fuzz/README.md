# Fuzz targets

Set up the same way (`cargo-fuzz` + `libfuzzer-sys`) as
[momokun7/xgrep](https://github.com/momokun7/xgrep)'s `rust/fuzz/` directory, but targeting
different code: the original project's fuzz targets (`fuzz_varint`, `fuzz_posting_list`,
`fuzz_index_reader`) exercise its on-disk binary index format, which muck doesn't
have — this server is purely in-memory. Instead these target the two places arbitrary
input actually reaches parsing/matching logic here:

- `fuzz_trigram_index` — `TrigramIndex::build`/`candidates` (src/trigram.rs) never panics and
  always returns valid, sorted, deduped doc ids for arbitrary document/query bytes.
- `fuzz_globfilter` — `GlobFilter::new`/`matches` (src/globfilter.rs, ported from the
  original project) never panics on arbitrary glob syntax or candidate paths.

## Running

```sh
cargo install cargo-fuzz
cd fuzz
cargo +nightly fuzz run fuzz_trigram_index
cargo +nightly fuzz run fuzz_globfilter
```
