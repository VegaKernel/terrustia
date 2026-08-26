//! Fuzzes `decode_section_stream` — packet 10, the tile-section decoder. README.md calls this
//! "the hardest part of the format", and it is the single largest thing this server ever sends
//! (about 45% of a session's bytes). It reads a run-length tile stream with no leading count byte
//! on a lone tile, then chest/sign/tile-entity trailers, entirely from attacker-controlled bytes —
//! this crate's own tests already prove specific truncation points don't panic; this explores the
//! space those hand-picked cases can't cover. `decode_section_stream` takes the *uncompressed*
//! body directly (the DEFLATE wrapper is a separate layer above it, in `write_section_stream`'s
//! sibling encoder) — no compression to get past, so raw fuzzer bytes reach real parsing
//! immediately, which is exactly why the corpus is seeded with genuine encoder output rather than
//! left to start from nothing: a fuzzer mutating a real stream finds the edges of real branches
//! far faster than one starting from noise.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = terrustia_proto::section::decode_section_stream(data);
});
