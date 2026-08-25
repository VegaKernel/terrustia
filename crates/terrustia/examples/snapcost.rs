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
    let world = match wld::load(&PathBuf::from(path)) {
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

    println!("\nmedian of nine runs, milliseconds:");
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
