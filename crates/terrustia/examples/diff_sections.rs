//! Decode two directories of captured tile-section payloads and report where their tiles differ.
//!
//! Byte differences alone cannot tell an encoding bug from a genuine content difference; this
//! compares the decoded tiles instead.

use std::{collections::BTreeMap, env, process::ExitCode};

use terrustia_proto::{
    Tile,
    section::{decode_section_stream, inflate_section_payload},
};

fn load(dir: &str) -> BTreeMap<(i32, i32), (i16, Vec<Tile>)> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for path in entries.filter_map(Result::ok).map(|e| e.path()) {
        if path.extension().is_none_or(|e| e != "deflate") {
            continue;
        }
        let Ok(payload) = std::fs::read(&path) else {
            continue;
        };
        let Ok(stream) = inflate_section_payload(&payload) else {
            continue;
        };
        let Ok((bounds, tiles, _)) = decode_section_stream(&stream) else {
            continue;
        };
        out.insert((bounds.x, bounds.y), (bounds.width, tiles));
    }
    out
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() < 2 {
        eprintln!("usage: diff_sections <dir a> <dir b>");
        return ExitCode::FAILURE;
    }
    let (a, b) = (load(&args[0]), load(&args[1]));

    let mut total = 0usize;
    let mut liquid_only = 0usize;
    for (key, (width, tiles_a)) in &a {
        let Some((_, tiles_b)) = b.get(key) else {
            continue;
        };
        for (i, (ta, tb)) in tiles_a.iter().zip(tiles_b).enumerate() {
            if ta == tb {
                continue;
            }
            total += 1;
            // Is the only difference the liquid?
            let mut normalised = *ta;
            normalised.liquid = tb.liquid;
            normalised.liquid_kind = tb.liquid_kind;
            if normalised == *tb {
                liquid_only += 1;
            } else if total - liquid_only <= 5 {
                let (x, y) = (
                    key.0 + (i % *width as usize) as i32,
                    key.1 + (i / *width as usize) as i32,
                );
                println!("  ({x}, {y})\n    a: {ta:?}\n    b: {tb:?}");
            }
        }
    }

    println!(
        "\n{total} differing tiles, of which {liquid_only} differ only in liquid ({} otherwise)",
        total - liquid_only
    );
    ExitCode::SUCCESS
}
