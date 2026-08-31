//! What does copying a world for the background save actually cost, and where?
//!
//! The tick's phase breakdown names `snapshot` as the most expensive thing an idle server does.
//! This says which part of the copy that is, so the fix goes where the time is rather than where
//! it looks like it should be.
//!
//! ```text
//! cargo run --release --example snapcost -- world.wld
//! ```

use std::{env, path::PathBuf, process::ExitCode, time::Instant};

use terrustia::world::wld;

/// Median of a handful of runs. A mean here is dominated by whichever run took a page fault.
fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("no NaNs from a clock"));
    samples[samples.len() / 2]
}

fn time<T>(runs: usize, mut work: impl FnMut() -> T) -> f64 {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let began = Instant::now();
        let made = work();
        samples.push(began.elapsed().as_secs_f64() * 1000.0);
        drop(std::hint::black_box(made));
    }
    median(samples)
}

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: snapcost <world.wld>");
        return ExitCode::FAILURE;
    };
    let mut world = match wld::load(&PathBuf::from(path)) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("load failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!(
        "{} ({}x{}), {} chests, {} signs, {} tile entities",
        world.name,
        world.width(),
        world.height(),
        world.chests.iter().flatten().count(),
        world.signs.iter().flatten().count(),
        world.tile_entities.len(),
    );

    let whole = time(9, || world.snapshot());
    let tiles = time(9, || world.tiles_for_measurement().clone());

    // The parts of the copy that are not tiles, and never change after load.
    let preserved = time(9, || world.preserved.clone());
    let objects = time(9, || {
        (
            world.chests.clone(),
            world.signs.clone(),
            world.town_npcs.clone(),
            world.tile_entities.clone(),
        )
    });

    // The loop above frees each copy before making the next, so the allocator hands back the same
    // warm pages every time. A live server does not: the previous snapshot is still being written
    // out, so each one is a fresh forty-megabyte mapping and the copy faults in every page of it.
    // Holding the copies apart reproduces that.
    let cold = {
        let mut held: Vec<terrustia::world::World> = Vec::new();
        let mut samples = Vec::new();
        for _ in 0..9 {
            let began = Instant::now();
            let copy = world.snapshot();
            samples.push(began.elapsed().as_secs_f64() * 1000.0);
            held.push(copy);
        }
        drop(std::hint::black_box(held));
        median(samples)
    };

    // And what it would cost to copy into a buffer we already own, which is the fix this points to.
    let reused = {
        let mut spare = world.snapshot();
        time(9, || {
            spare.copy_state_from(&world);
        })
    };

    // What a refresh costs with nothing dirty: the side tables and the object tables, which are
    // copied wholesale on every call however few sections changed. This is the floor under any
    // scheme that spreads the tile copying over several ticks.
    let fixed = {
        let mut spare = world.snapshot();
        time(9, || world.refresh_snapshot(&mut spare))
    };

    // What copying only the tiles that actually changed would cost, against copying the sections
    // they happen to sit in. An idle server changes on the order of two hundred tiles between
    // saves, and that marks twenty-four to thirty-seven sections: a section is 200x150 = 30,000
    // tiles, so two hundred real changes drag about a million tiles into the copy.
    //
    // Scattered so no two picks share a cache line, which is the worst case for a tile list; and
    // through the public `tile`/`set_tile` pair, which is dearer than the raw index copy a real
    // implementation would do (it rebuilds each `Tile` from the side tables and then takes it
    // apart again). Both make this an upper bound on what a tile list would cost.
    let scattered = |n: usize| -> f64 {
        let mut spare = world.snapshot();
        let (w, h) = (world.width() as u64, world.height() as u64);
        let picks: Vec<(i32, i32)> = (0..n as u64)
            .map(|i| {
                let at = i.wrapping_mul(2_654_435_761) % (w * h);
                ((at % w) as i32, (at / w) as i32)
            })
            .collect();
        time(9, || {
            for &(x, y) in &picks {
                spare.set_tile(x, y, world.tile(x, y));
            }
        })
    };
    let (scattered_200, scattered_4k) = (scattered(200), scattered(4_000));

    // And what it costs per section. A section is a 200x150 rectangle of a row-major array, so
    // copying one is 150 short strided memcpys rather than a single long one: the per-tile rate is
    // far worse than the contiguous copy above, and that difference is the whole autosave spike.
    let sections = (world.sections_x() * world.sections_y()) as f64;
    let per_section = {
        let mut spare = world.snapshot();
        world.start_tracking_changes();
        time(9, || {
            for sy in 0..world.sections_y() {
                for sx in 0..world.sections_x() {
                    let (x, y) = (sx * 200, sy * 150);
                    world.set_tile(x, y, world.tile(x, y));
                }
            }
            world.refresh_snapshot(&mut spare)
        })
    };

    println!("\nmedian of nine runs, milliseconds:");
    println!("  refresh, 0 dirty {fixed:8.3}  (the fixed cost of a refresh: side + object tables)");
    println!(
        "  refresh, all {sections:3.0}  {per_section:8.3}  ({:.0} us per section)",
        per_section * 1000.0 / sections
    );
    println!("  200 loose tiles  {scattered_200:8.3}  (what an idle window actually changes)");
    println!("  4000 loose tiles {scattered_4k:8.3}");
    println!("  whole snapshot   {whole:8.3}  (allocator handing back warm pages)");
    println!("  ... allocating   {cold:8.3}  (a fresh mapping each time, as the server does)");
    println!("  ... into a spare {reused:8.3}  (reusing a buffer we already own)");
    println!("  tiles            {tiles:8.3}");
    println!("  preserved bytes  {preserved:8.3}");
    println!("  chests/signs/etc {objects:8.3}");
    println!(
        "\nA tick's budget is 16.667 ms. The snapshot runs on the game task, so anything here is \n\
         time the world is not being simulated in."
    );
    ExitCode::SUCCESS
}
