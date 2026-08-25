//! Measure what a world save actually costs, split into its two halves.
//!
//! A save runs on the tick thread, and the tick has 16,666 µs to spend. If a save is 70 ms then
//! every autosave drops four ticks, which players feel as a stutter. Whether that can be fixed by
//! moving work off the thread depends entirely on *which* half is slow: serialising needs the
//! world and must stay where it is, writing does not.
//!
//! ```sh
//! cargo run --release -p terrustia --example savecost -- [world.wld]
//! ```
//!
//! With no argument it generates a large world instead, which is the interesting case — a small
//! one saves fast enough that the problem does not appear.

use std::{env, process::ExitCode, time::Instant};

use terrustia::world::{wld, wld_save, worldgen};

fn main() -> ExitCode {
    let world = match env::args().nth(1) {
        Some(path) => {
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    eprintln!("could not read {path}: {e}");
                    return ExitCode::FAILURE;
                }
            };
            match wld::parse(&bytes) {
                Ok(world) => world,
                Err(e) => {
                    eprintln!("could not parse {path}: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        None => {
            println!("generating a large world (4200x1200)...");
            worldgen::generate(4200, 1200, String::from("savecost"), 1234)
        }
    };

    println!(
        "world       {} ({}x{}), {} tiles",
        world.name,
        world.width(),
        world.height(),
        world.width() as i64 * world.height() as i64
    );

    // Warm, so the first run's page faults do not dominate.
    let _ = wld_save::serialize(&world);

    let mut serialise_us = Vec::new();
    let mut bytes_len = 0;
    for _ in 0..5 {
        let began = Instant::now();
        let bytes = wld_save::serialize(&world).expect("a save");
        serialise_us.push(began.elapsed().as_micros() as u64);
        bytes_len = bytes.len();
    }

    let temp = std::env::temp_dir().join("terrustia-savecost.wld");
    let bytes = wld_save::serialize(&world).expect("a save");
    let mut write_us = Vec::new();
    for _ in 0..5 {
        let began = Instant::now();
        std::fs::write(&temp, &bytes).expect("a write");
        write_us.push(began.elapsed().as_micros() as u64);
    }
    std::fs::remove_file(&temp).ok();

    let median = |mut v: Vec<u64>| {
        v.sort_unstable();
        v[v.len() / 2]
    };
    let s = median(serialise_us);
    let w = median(write_us);

    // Where does the time inside a serialise actually go? Guessing wasted an afternoon once
    // already: a transpose that should have fixed a cache problem changed nothing, because the
    // cache was not the problem. So each layer is timed on its own.
    let (width, height) = (world.width(), world.height());
    let mut read_us = Vec::new();
    for _ in 0..3 {
        let began = Instant::now();
        let mut sum = 0u64;
        for x in 0..width {
            for y in 0..height {
                sum += u64::from(world.tile(x, y).block);
            }
        }
        std::hint::black_box(sum);
        read_us.push(began.elapsed().as_micros() as u64);
    }
    let mut run_us = Vec::new();
    for _ in 0..3 {
        let began = Instant::now();
        let mut runs = 0u64;
        for x in 0..width {
            let mut pending: Option<terrustia_proto::Tile> = None;
            for y in 0..height {
                let tile = world.tile(x, y);
                match pending {
                    Some(prev)
                        if prev == tile
                            && terrustia_proto::tile_sets::allows_batching(tile.block) => {}
                    _ => {
                        runs += 1;
                        pending = Some(tile);
                    }
                }
            }
        }
        std::hint::black_box(runs);
        run_us.push(began.elapsed().as_micros() as u64);
    }
    let r = median(read_us);
    let rr = median(run_us);

    println!();
    println!("  reading every tile and nothing else   {r:>8} µs");
    println!("  ...plus finding the runs              {rr:>8} µs");
    println!("  ...plus encoding them (the whole job) {s:>8} µs");
    println!(
        "  so encoding is                        {:>8} µs of it",
        s.saturating_sub(rr)
    );

    println!("file        {} bytes", bytes_len);
    println!();
    println!(
        "serialise   {:>8} µs   (needs the world; must stay on the tick)",
        s
    );
    println!("write       {:>8} µs   (can move off the tick)", w);
    println!("total       {:>8} µs", s + w);
    println!();
    println!();
    println!("tick budget {:>8} µs", 16_666);
    println!("a save is   {:>8.2}x a tick", (s + w) as f64 / 16_666.0);
    println!(
        "per tile    {:>8.1} ns",
        (s as f64 * 1000.0) / (world.width() as f64 * world.height() as f64)
    );
    println!();
    if s + w > 16_666 {
        println!("A save busts the tick budget, so every autosave drops ticks and players see a");
        println!("stutter. Either the encoder gets faster or the work moves off the thread.");
    } else {
        println!("A save fits inside one tick, so an autosave costs nothing anybody can see.");
    }
    ExitCode::SUCCESS
}
