//! Load a `.wld` file and report what came out of it.
//!
//! ```text
//! cargo run --release --example load_wld -- "$HOME/Library/Application Support/Terraria/Worlds/My.wld"
//! ```

use std::{collections::BTreeMap, env, path::PathBuf, process::ExitCode, time::Instant};

use terrustia::world::wld;

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: load_wld <path to .wld>");
        return ExitCode::FAILURE;
    };

    let started = Instant::now();
    let world = match wld::load(&path) {
        Ok(world) => world,
        Err(e) => {
            eprintln!("failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let elapsed = started.elapsed();

    println!("name        {}", world.name);
    println!("size        {} x {}", world.width(), world.height());
    println!("spawn       ({}, {})", world.spawn_x, world.spawn_y);
    println!("surface     {}", world.surface);
    println!(
        "dungeon     ({}, {})",
        world
            .dungeon_x
            .map_or_else(|| "?".into(), |x| x.to_string()),
        world
            .dungeon_y
            .map_or_else(|| "?".into(), |y| y.to_string()),
    );
    let p = world.progress;
    println!(
        "progress    hardmode={} altars={} orbs={} eye={} evil_boss={} skeletron={} queen_bee={}",
        p.hard_mode,
        p.altar_count,
        p.shadow_orb_count,
        p.downed_boss1,
        p.downed_boss2,
        p.downed_boss3,
        p.downed_queen_bee
    );
    println!(
        "late        cultist={} moon_lord={} fishron={} martians={} towers={:?} apocalypse={}",
        p.downed_ancient_cultist,
        p.downed_moon_lord,
        p.downed_fishron,
        p.downed_martians,
        (
            p.downed_tower_solar,
            p.downed_tower_vortex,
            p.downed_tower_nebula,
            p.downed_tower_stardust
        ),
        p.lunar_apocalypse_up,
    );
    println!(
        "weather     raining={} rain_time={} max_rain={:.2} wind={:.3}",
        world.raining, world.rain_time, world.max_rain, world.wind,
    );
    println!(
        "            mechs={}/{}/{} any={} plantera={} golem={} king_slime={}",
        p.downed_mech1,
        p.downed_mech2,
        p.downed_mech3,
        p.downed_mech_any,
        p.downed_plantera,
        p.downed_golem,
        p.downed_king_slime
    );
    println!(
        "            saved goblin={} wizard={} mechanic={} | goblins={} clown={} frost={} pirates={}",
        p.saved_goblin,
        p.saved_wizard,
        p.saved_mechanic,
        p.downed_goblins,
        p.downed_clown,
        p.downed_frost,
        p.downed_pirates
    );
    println!("rock layer  {}", world.rock_layer);
    println!("world id    {}", world.id);
    println!("game mode   {}", world.game_mode);
    println!(
        "evil        {}",
        if world.crimson {
            "crimson"
        } else {
            "corruption"
        }
    );
    println!(
        "time        {} ({})",
        world.time,
        if world.day_time { "day" } else { "night" }
    );
    let chests: Vec<_> = world.chests.iter().flatten().collect();
    let signs: Vec<_> = world.signs.iter().flatten().collect();
    let stored: usize = chests
        .iter()
        .map(|c| c.items.iter().filter(|i| !i.is_empty()).count())
        .sum();
    println!("chests      {} ({stored} item stacks inside)", chests.len());
    println!("signs       {}", signs.len());
    println!("entities    {}", world.tile_entities.len());
    if !world.tile_entities.is_empty() {
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        for entity in &world.tile_entities {
            *kinds.entry(format!("{:?}", entity.kind)).or_default() += 1;
        }
        for (kind, count) in kinds {
            println!("              {count:>5}  {kind}");
        }
    }
    if let Some(p) = &world.preserved {
        println!(
            "preserved   header {} B, trailing sections {} ({} B)",
            p.header_bytes.len(),
            p.trailing_sections.len(),
            p.trailing_sections.iter().map(Vec::len).sum::<usize>()
        );
    }
    println!("loaded in   {} ms", elapsed.as_millis());

    let mut blocks: BTreeMap<u16, usize> = BTreeMap::new();
    let mut walls: BTreeMap<u16, usize> = BTreeMap::new();
    let mut active = 0usize;
    for y in 0..world.height() {
        for x in 0..world.width() {
            let tile = world.tile(x, y);
            if tile.is_active() {
                active += 1;
                *blocks.entry(tile.block).or_default() += 1;
            }
            if tile.wall != 0 {
                *walls.entry(tile.wall).or_default() += 1;
            }
        }
    }

    let total = (world.width() as usize) * (world.height() as usize);
    println!(
        "tiles       {total} ({active} active, {:.1}%)",
        active as f64 / total as f64 * 100.0
    );

    let mut top: Vec<_> = blocks.into_iter().collect();
    top.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("top blocks  {:?}", &top[..top.len().min(8)]);

    let mut top_walls: Vec<_> = walls.into_iter().collect();
    top_walls.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("top walls   {:?}", &top_walls[..top_walls.len().min(6)]);

    if let Some(chest) = chests
        .iter()
        .find(|c| c.items.iter().any(|i| !i.is_empty()))
    {
        let contents: Vec<_> = chest
            .items
            .iter()
            .filter(|i| !i.is_empty())
            .take(4)
            .map(|i| format!("{}x item {}", i.stack, i.id))
            .collect();
        println!(
            "a chest     ({}, {}) {} slots: {}",
            chest.x,
            chest.y,
            chest.items.len(),
            contents.join(", ")
        );
    }
    if let Some(sign) = signs.first() {
        println!("a sign      ({}, {}) {:?}", sign.x, sign.y, sign.text);
    }
    ExitCode::SUCCESS
}
