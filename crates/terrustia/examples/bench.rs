//! Time the hot paths: world load, section encoding, and the tile scan the AI does.

use std::{env, path::PathBuf, time::Instant};

use terrustia::world::{World, wld, worldgen};
use terrustia_proto::{
    section::{encode_section_packet, inflate_section_payload},
    tile_sets::frame_important,
};

fn main() {
    let world: World = match env::args().nth(1) {
        Some(path) => {
            let started = Instant::now();
            let w = wld::load(&PathBuf::from(path)).expect("load");
            println!(
                "world load            {:>8.1} ms",
                started.elapsed().as_secs_f64() * 1e3
            );
            w
        }
        None => {
            let started = Instant::now();
            let w = worldgen::generate(4200, 1200, "bench", 1);
            println!(
                "world generate        {:>8.1} ms",
                started.elapsed().as_secs_f64() * 1e3
            );
            w
        }
    };

    // How many tiles actually need frame data? That decides whether frames can move to a side
    // table and shrink every tile by four bytes.
    let mut framed = 0usize;
    let mut active = 0usize;
    let mut with_liquid = 0usize;
    for y in 0..world.height() {
        for x in 0..world.width() {
            let t = world.tile(x, y);
            if t.is_active() {
                active += 1;
                if frame_important(t.block) {
                    framed += 1;
                }
            }
            if t.liquid > 0 {
                with_liquid += 1;
            }
        }
    }
    let total = world.width() as usize * world.height() as usize;
    println!(
        "tiles                 {total}  active {active} ({:.0}%)  framed {framed} ({:.2}%)  liquid {with_liquid}",
        active as f64 / total as f64 * 100.0,
        framed as f64 / total as f64 * 100.0
    );

    // Section encoding, which every connecting client pays for 15 times over.
    let (sx, sy) = world.section_of(i32::from(world.spawn_x), i32::from(world.spawn_y));
    let bounds = world.section_bounds(sx, sy);
    let extras = world.extras_for(bounds);

    let runs = 200;
    let started = Instant::now();
    let mut bytes = 0usize;
    for _ in 0..runs {
        let frame = encode_section_packet(bounds, &extras, |x, y| world.tile(x, y)).unwrap();
        bytes = frame.len();
    }
    let per = started.elapsed().as_secs_f64() / f64::from(runs) * 1e3;
    println!("section encode        {per:>8.3} ms   ({bytes} bytes out)");
    println!("  15 sections (a join){:>8.1} ms", per * 15.0);

    let frame = encode_section_packet(bounds, &extras, |x, y| world.tile(x, y)).unwrap();
    let started = Instant::now();
    for _ in 0..runs {
        let _ = inflate_section_payload(&frame[3..]).unwrap();
    }
    println!(
        "section inflate       {:>8.3} ms",
        started.elapsed().as_secs_f64() / f64::from(runs) * 1e3
    );

    // A full save, which happens on autosave and shutdown.
    if world.preserved.is_some() {
        let started = Instant::now();
        let bytes = terrustia::world::wld_save::serialize(&world).unwrap();
        println!(
            "world serialise       {:>8.1} ms   ({:.1} MB)",
            started.elapsed().as_secs_f64() * 1e3,
            bytes.len() as f64 / 1e6
        );
    }
}
