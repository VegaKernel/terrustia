//! Decode tile-section payloads captured from a real Terraria server.
//!
//! This is the strongest check available for `terrustia_proto::section`: a round-trip only proves
//! our encoder and decoder agree with each other, whereas decoding the shipping game's own output
//! proves we agree with *it*.
//!
//! ```text
//! PROBE_DUMP_DIR=/tmp/sections cargo run --example probe -- 127.0.0.1:7778
//! cargo run --example verify_sections -- /tmp/sections
//! ```

use std::{collections::BTreeMap, env, process::ExitCode};

use terrustia_proto::{
    Writer,
    section::{decode_section_stream, inflate_section_payload, write_section_stream},
};

fn main() -> ExitCode {
    let Some(dir) = env::args().nth(1) else {
        eprintln!("usage: verify_sections <directory of .deflate payloads>");
        return ExitCode::FAILURE;
    };

    let mut files: Vec<_> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries.filter_map(Result::ok).map(|e| e.path()).collect(),
        Err(e) => {
            eprintln!("{dir}: {e}");
            return ExitCode::FAILURE;
        }
    };
    files.sort();
    files.retain(|p| p.extension().is_some_and(|e| e == "deflate"));

    if files.is_empty() {
        eprintln!("no .deflate payloads in {dir}");
        return ExitCode::FAILURE;
    }

    let mut failures = 0usize;
    let mut tile_histogram: BTreeMap<u16, usize> = BTreeMap::new();

    for path in &files {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let payload = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                println!("{name}: unreadable: {e}");
                failures += 1;
                continue;
            }
        };

        let stream = match inflate_section_payload(&payload) {
            Ok(stream) => stream,
            Err(e) => {
                println!("{name}: inflate failed: {e}");
                failures += 1;
                continue;
            }
        };

        match decode_section_stream(&stream) {
            Ok((bounds, tiles, extras)) => {
                let active = tiles.iter().filter(|t| t.is_active()).count();
                for tile in tiles.iter().filter(|t| t.is_active()) {
                    *tile_histogram.entry(tile.block).or_default() += 1;
                }

                // Re-encoding must reproduce the byte stream exactly. Anything else means we
                // decoded correctly but would emit something the client reads differently.
                let mut re = Writer::new();
                write_section_stream(&mut re, bounds, &extras, |x, y| {
                    let ix = (x - bounds.x) as usize;
                    let iy = (y - bounds.y) as usize;
                    tiles[iy * bounds.width as usize + ix]
                });
                let identical = re.as_slice() == stream.as_slice();

                println!(
                    "{name}: ok  ({},{}) {}x{}  {} tiles, {active} active, {} chests, {} signs  re-encode {}",
                    bounds.x,
                    bounds.y,
                    bounds.width,
                    bounds.height,
                    tiles.len(),
                    extras.chests.len(),
                    extras.signs.len(),
                    if identical {
                        "byte-identical".to_string()
                    } else {
                        failures += 1;
                        format!("DIFFERS ({} vs {} bytes)", re.len(), stream.len())
                    }
                );
            }
            Err(e) => {
                println!(
                    "{name}: decode failed: {e}  (inflated {} bytes)",
                    stream.len()
                );
                failures += 1;
            }
        }
    }

    println!("\n{} section(s), {failures} failure(s)", files.len());
    let mut top: Vec<_> = tile_histogram.into_iter().collect();
    top.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    println!("most common tile types: {:?}", &top[..top.len().min(8)]);

    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
