//! What the biome scan costs the spawn tick, with and without `BiomeCache`. Dev tool, not part of
//! the server: it exists to check that reading the zone on every spawn attempt (which
//! `NPC.GetSpawnRate` requires) still fits in a tick at the 255-player bar.
//!
//! **This measures the worst tick, not the mean, and that distinction is the whole point.** An
//! earlier version drove a single slot for 60,000 ticks and multiplied its per-tick mean by 255.
//! That reported 345 us and was arithmetically correct: 255 players over a 60-tick refresh really is
//! 4.25 scans a tick. But multiplying a mean by the player count assumes the work spreads evenly,
//! and it does not. Clients arrive in a burst, so every slot fills on the same tick and every entry
//! then expires on the same tick. A real 255-player soak measured `phase=spawning phase_us=20763`,
//! which is 266 scans in one tick, over the entire frame budget, while this example was still
//! reporting 345 us. A per-tick mean over one player cannot see a per-tick maximum over 255.

//! It also measures the two desert checks, because they are the only other zone tests on this path
//! and the interesting thing about them is that they are not scans at all: the underground desert
//! is a wall set (`NPC.cs:1682`) and a sandstorm spot is a short sand walk (`NPC.cs:5464-5503`),
//! so both are answered per candidate tile where the biome scan has to be cached per player.

use terrustia::game::spawn::{
    BiomeCache, SPAWN_RANGE_X, SPAWN_RANGE_Y, biome_at, sandstone_check, underground_desert_spot,
};
use terrustia::world::worldgen;

/// Players in the burst, matching the server's own qualification bar.
const PLAYERS: usize = 255;

/// Candidate tiles one spawn attempt tries before giving up (`try_spawn`'s own `0..20`).
const CANDIDATES: u32 = 20;

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

    // The join burst: every slot reads for the first time on tick 1, so every entry carries the
    // same age and they all come due together. Held long enough to cross several refresh windows.
    let mut cache = BiomeCache::default();
    let ticks = 600u64;
    let mut worst = 0.0f64;
    let mut worst_tick = 0u64;
    let mut total = 0.0f64;
    for tick in 1..=ticks {
        let start = std::time::Instant::now();
        cache.advance(tick);
        for slot in 0..PLAYERS {
            std::hint::black_box(cache.read(&world, slot, x, y));
        }
        let us = start.elapsed().as_secs_f64() * 1e6;
        total += us;
        if us > worst {
            worst = us;
            worst_tick = tick;
        }
    }
    let mean = total / ticks as f64;

    println!("biome_at             : {raw:.1} us per scan");
    println!(
        "  {PLAYERS} players, uncached: {:.0} us in one tick",
        raw * f64::from(u32::try_from(PLAYERS).unwrap_or(255))
    );
    println!();
    println!("cached, {PLAYERS} players in one burst, over {ticks} ticks:");
    println!("  mean per tick      : {mean:.1} us");
    println!("  WORST tick         : {worst:.1} us  (tick {worst_tick})");
    println!();
    println!("tick budget          : 16666.7 us");
    println!("worst tick is {:.2}% of budget", worst / 16666.7 * 100.0);

    // The two desert checks, over the same spread of tiles a spawn attempt actually walks. Both are
    // per candidate rather than per player, so the number that matters is the cost of a whole
    // attempt: 20 candidates. Each is the best of seven runs, and so is the `biome_at` it is
    // compared against, because a machine running anything else at the same time moves an average
    // far more than it moves a minimum.
    let spread = |i: i32| {
        (
            x + i % (SPAWN_RANGE_X * 2) - SPAWN_RANGE_X,
            y + i % SPAWN_RANGE_Y,
        )
    };
    let runs = 200_000;
    let best = |mut f: Box<dyn FnMut() -> f64>| (0..7).map(|_| f()).fold(f64::MAX, f64::min);
    let per = |start: std::time::Instant| start.elapsed().as_secs_f64() / f64::from(runs) * 1e6;

    // The ordinary case: no desert wall anywhere, so `SpawnTileOrAboveHasAnyWallInSet` answers in
    // two tile reads and `WorldGen.checkUnderground` is never reached at all.
    let ordinary = best(Box::new(|| {
        let start = std::time::Instant::now();
        for i in 0..runs {
            let (cx, cy) = spread(i);
            std::hint::black_box(underground_desert_spot(&world, cx, cy));
        }
        per(start)
    }));

    // The worst case, which has to be built and is narrower than it looks. `checkUnderground` walks
    // its 120-by-3 strip only when the point is above the `worldSurface + 80` shortcut *and* has no
    // wall of its own, and `underground_desert_spot` only calls it at all once a desert wall has
    // been found. So the one arrangement that pays for the strip is a desert wall on the row above
    // the ground with a bare ground tile under it: the wall on alternating rows below, with the
    // probe driven onto the rows that see it.
    let mut walled = worldgen::generate(4200, 1200, "biome scan cost", 7);
    let band = i32::from(walled.surface) + 40;
    for cx in x - SPAWN_RANGE_X - 2..=x + SPAWN_RANGE_X + 2 {
        for cy in band - SPAWN_RANGE_Y - 2..=band + SPAWN_RANGE_Y + 2 {
            let mut tile = walled.tile(cx, cy);
            tile.wall = if cy % 2 == 0 { 187 } else { 0 }; // Sandstone on the upper row only
            walled.set_tile(cx, cy, tile);
        }
        // ...and a closed roof eighty tiles above, which is the thing the strip is counting. Without
        // it the walk still happens but always answers false, which measures the same loop while
        // quietly testing a case that never reaches the roster.
        for cy in band - 84..=band - 70 {
            walled.set_tile(cx, cy, terrustia_proto::Tile::block(1)); // Stone
        }
    }
    let worst_desert = best(Box::new(|| {
        let start = std::time::Instant::now();
        for i in 0..runs {
            let (cx, _) = spread(i);
            std::hint::black_box(underground_desert_spot(&walled, cx, band + (i % 4) * 2));
        }
        per(start)
    }));
    assert!(
        underground_desert_spot(&walled, x, band),
        "the worst-case world has to actually read as an underground desert",
    );

    let sandstone = best(Box::new(|| {
        let start = std::time::Instant::now();
        for i in 0..runs {
            let (cx, cy) = spread(i);
            std::hint::black_box(sandstone_check(&world, cx, cy));
        }
        per(start)
    }));
    let scan = best(Box::new(|| {
        let start = std::time::Instant::now();
        for i in 0..500 {
            std::hint::black_box(biome_at(&world, x + (i % 17), y));
        }
        start.elapsed().as_secs_f64() / 500.0 * 1e6
    }));
    let attempt = (worst_desert + sandstone) * f64::from(CANDIDATES);

    println!();
    println!("best of 7 runs each, `biome_at` measured the same way for comparison:");
    println!("  biome_at                       : {scan:.1} us per scan");
    println!("  underground_desert_spot        : {ordinary:.3} us per candidate (no desert wall)");
    println!("  ...with checkUnderground walked: {worst_desert:.3} us per candidate (worst case)");
    println!("  sandstone_check                : {sandstone:.3} us per candidate");
    println!(
        "  worst case over {CANDIDATES} candidates: {attempt:.2} us, {:.0}x cheaper than one scan",
        scan / attempt,
    );
    println!(
        "  ...but a candidate that reaches these has already passed every earlier gate, and in an\n  \
         underground desert that candidate then spawns something and breaks the loop. One per\n  \
         attempt is the realistic figure, so {:.2} us, and only for an attempt that passed the rate\n  \
         roll: about one tick in 600 per player.",
        worst_desert + sandstone,
    );
    let _ = (raw, mean);
}
