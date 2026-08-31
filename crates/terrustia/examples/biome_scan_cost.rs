//! What the biome scan costs the spawn tick, with and without `BiomeCache`. Dev tool, not part of
//! the server: it exists to check that reading the zone on every spawn attempt (which
//! `NPC.GetSpawnRate` requires) still fits in a tick at the 255-player bar.

use terrustia::game::spawn::{BiomeCache, biome_at};
use terrustia::world::worldgen;

fn main() {
    let world = worldgen::generate(4200, 1200, "biome scan cost", 7);
    let x = world.width() / 2;
    let y = i32::from(world.surface) + 40;

    for _ in 0..20 {
        std::hint::black_box(biome_at(&world, x, y));
    }

    let runs = 500;
    let start = std::time::Instant::now();
    for i in 0..runs {
        std::hint::black_box(biome_at(&world, x + (i % 17), y));
    }
    let raw = start.elapsed().as_secs_f64() / f64::from(runs) * 1e6;

    // A standing player, read once per tick for a second: what the cache actually costs.
    let mut cache = BiomeCache::default();
    let ticks = 60_000u64;
    let start = std::time::Instant::now();
    for tick in 0..ticks {
        cache.advance(tick);
        std::hint::black_box(cache.read(&world, 0, x, y));
    }
    let cached = start.elapsed().as_secs_f64() / ticks as f64 * 1e6;

    println!("biome_at            : {raw:.1} us per scan");
    println!("  255 players, raw  : {:.1} us per tick", raw * 255.0);
    println!("cached read         : {cached:.2} us per tick per player");
    println!("  255 players, cached: {:.1} us per tick", cached * 255.0);
    println!("tick budget         : 16666.7 us");
}
