//! Report where each biome is in a world, so a spawn check can be pointed at one.
//!
//! Natural spawning is biome-driven, and a table that names the wrong creature for a place looks
//! right until somebody stands there. This finds a place to stand.

use std::{env, process::ExitCode};

use terrustia::{
    game::spawn::{Biome, biome_at, depth_at},
    world::wld,
};

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: biomes <world.wld>");
        return ExitCode::FAILURE;
    };
    let world = match wld::load(&std::path::PathBuf::from(path)) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("load failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!("{} ({}x{})", world.name, world.width(), world.height());

    // A count of the blocks that define each evil and the hallow, so a world with none of one is
    // obvious rather than looking like a classifier bug.
    let mut marks = std::collections::BTreeMap::new();
    for x in 0..world.width() {
        for y in 0..world.height() {
            let block = world.tile(x, y).block;
            let what = match block {
                23 | 25 | 112 | 163 | 398 | 400 => "corruption",
                199 | 200 | 203 | 234 | 399 | 401 => "crimson",
                109 | 116 | 117 | 164 | 402 | 403 => "hallow",
                _ => continue,
            };
            *marks.entry(what).or_insert(0u32) += 1;
        }
    }
    for (what, count) in &marks {
        println!("  {what:<12} {count} tiles");
    }

    // One sample every fifty tiles across, at three depths.
    let surface = i32::from(world.surface);
    let mut first: std::collections::BTreeMap<String, (i32, i32)> =
        std::collections::BTreeMap::new();
    for x in (300..world.width() - 300).step_by(50) {
        for y in [surface - 10, surface + 100, surface + 400] {
            if y < 10 || y >= world.height() - 40 {
                continue;
            }
            let biome = biome_at(&world, x, y);
            if biome == Biome::Forest {
                continue;
            }
            let key = format!("{biome:?} {:?}", depth_at(&world, y));
            first.entry(key).or_insert((x, y));
        }
    }
    for (what, (x, y)) in first {
        println!("  {what:<24} at {x}, {y}");
    }
    ExitCode::SUCCESS
}
