//! Check a world holds everything a playthrough needs.
//!
//! Terrain alone is scenery. This walks a `.wld` and reports whether each link in the progression
//! chain is actually present — and says which boss is unreachable when one is not.
//!
//! ```sh
//! cargo run --release -p terrustia --example playable -- world.wld
//! ```

use std::{collections::HashMap, env, process::ExitCode};

use terrustia::world::wld;

/// Each thing a world must contain, and what it gates.
const NEEDED: &[(u16, &str, &str)] = &[
    (
        31,
        "shadow orbs / crimson hearts",
        "Eater of Worlds or Brain of Cthulhu",
    ),
    (
        26,
        "demon altars",
        "hardmode ores, so every mechanical boss",
    ),
    (12, "life crystals", "more than a hundred hit points"),
    (21, "chests", "starter weapons, hooks and boots"),
    (58, "hellstone", "the Wall of Flesh, so hardmode"),
    (226, "lihzahrd brick", "the Golem"),
    (
        60,
        "jungle grass",
        "the jungle, so Plantera and the Queen Bee",
    ),
    (231, "larva", "the Queen Bee without a summon"),
];

/// The dungeon, which is any one of three bricks.
const DUNGEON: [u16; 3] = [41, 43, 44];
/// The evil's stone, either kind.
const EVIL_STONE: [u16; 2] = [25, 203];

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: playable <world.wld>");
        return ExitCode::FAILURE;
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!("could not read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let world = match wld::parse(&bytes) {
        Ok(world) => world,
        Err(e) => {
            eprintln!("could not parse {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut census: HashMap<u16, usize> = HashMap::new();
    for x in 0..world.width() {
        for y in 0..world.height() {
            let tile = world.tile(x, y);
            if tile.is_active() {
                *census.entry(tile.block).or_default() += 1;
            }
        }
    }

    println!("{} ({}x{})", world.name, world.width(), world.height());
    println!(
        "  {} · spawn {},{} · surface {} · rock {}",
        if world.crimson {
            "crimson"
        } else {
            "corruption"
        },
        world.spawn_x,
        world.spawn_y,
        world.surface,
        world.rock_layer
    );
    // Which ore each tier settled on. The hardmode three read "unchosen" until the first three
    // altars are broken; a 0 there is the bug that makes an altar spray dirt instead of ore.
    let ore = |v: i16| match v {
        -1 => "unchosen".to_string(),
        other => other.to_string(),
    };
    println!(
        "  ore: copper {} iron {} silver {} gold {} | cobalt {} mythril {} adamantite {}",
        ore(world.ore_tiers[0]),
        ore(world.ore_tiers[1]),
        ore(world.ore_tiers[2]),
        ore(world.ore_tiers[3]),
        ore(world.ore_tiers[4]),
        ore(world.ore_tiers[5]),
        ore(world.ore_tiers[6]),
    );
    println!();

    let mut missing = 0;
    let report = |count: usize, what: &str, gates: &str| {
        if count > 0 {
            println!("  {count:>8}  {what}");
        } else {
            println!("       --  {what}   MISSING: no {gates}");
        }
        usize::from(count == 0)
    };

    for &(tile, what, gates) in NEEDED {
        missing += report(census.get(&tile).copied().unwrap_or(0), what, gates);
    }
    let dungeon: usize = DUNGEON
        .iter()
        .map(|t| census.get(t).copied().unwrap_or(0))
        .sum();
    missing += report(
        dungeon,
        "dungeon brick",
        "Skeletron, and everything behind him",
    );
    let evil: usize = EVIL_STONE
        .iter()
        .map(|t| census.get(t).copied().unwrap_or(0))
        .sum();
    missing += report(evil, "evil stone", "evil biome");
    missing += report(
        world.chests.iter().flatten().count(),
        "chests with contents",
        "loot",
    );

    println!();
    if missing == 0 {
        println!("this world can be played through: every link in the chain is present.");
        ExitCode::SUCCESS
    } else {
        println!("{missing} of the chain's links are missing; this world cannot be finished.");
        ExitCode::FAILURE
    }
}
