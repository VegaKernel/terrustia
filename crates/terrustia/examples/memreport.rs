//! Report where the server's memory actually goes.

use std::{env, mem::size_of, path::PathBuf};

use terrustia::world::{World, wld, worldgen};
use terrustia_proto::Tile;

fn rss_mb() -> f64 {
    // macOS: ask ps for our own resident set.
    let pid = std::process::id();
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok();
    out.and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map(|kb| kb / 1024.0)
        .unwrap_or(f64::NAN)
}

fn report(label: &str, world: &World) {
    let tiles = world.width() as usize * world.height() as usize;
    let (array, frames, paint) = world.tile_footprint();
    let bytes = array + frames + paint;
    println!("{label}");
    println!(
        "  size_of::<Tile>()   {} bytes (the value type callers see)",
        size_of::<Tile>()
    );
    println!(
        "  packed in the array {} bytes",
        size_of::<terrustia::world::packed::PackedTile>()
    );
    println!(
        "  tiles               {tiles} ({}x{})",
        world.width(),
        world.height()
    );
    println!("  tile array          {:.1} MB", array as f64 / 1e6);
    println!("  frame side table    {:.2} MB", frames as f64 / 1e6);
    println!("  paint side table    {:.2} MB", paint as f64 / 1e6);
    println!("  tiles, all in       {:.1} MB", bytes as f64 / 1e6);
    println!("  the same at 16 B    {:.1} MB", (tiles * 16) as f64 / 1e6);
    println!("  process RSS         {:.1} MB", rss_mb());

    // Touch every tile so nothing is left lazily unmapped, then measure again.
    let mut active = 0usize;
    for y in 0..world.height() {
        for x in 0..world.width() {
            if world.tile(x, y).is_active() {
                active += 1;
            }
        }
    }
    println!(
        "  after full scan     {:.1} MB   ({active} active tiles)",
        rss_mb()
    );

    // What a leaner layout would cost.
    println!("  if Tile were 8 B    {:.1} MB", (tiles * 8) as f64 / 1e6);
    println!("  if Tile were 6 B    {:.1} MB", (tiles * 6) as f64 / 1e6);

    // How much of the struct is only ever needed by a minority of tiles.
    //
    // Frames and paint are the two candidates for moving out of the tile and into a side table:
    // both cost every tile in the world four and two bytes respectively, and both are meaningful
    // for a fraction of them. The question is what fraction, because a side table is only cheaper
    // than the bytes it saves if it stays small.
    let mut framed = 0usize;
    let mut painted = 0usize;
    let mut sloped = 0usize;
    for y in 0..world.height() {
        for x in 0..world.width() {
            let tile = world.tile(x, y);
            if tile.is_active() && terrustia_proto::tile_sets::frame_important(tile.block) {
                framed += 1;
            }
            if tile.color != 0 || tile.wall_color != 0 {
                painted += 1;
            }
            if tile.slope != 0 {
                sloped += 1;
            }
        }
    }
    let share = |n: usize| n as f64 / tiles as f64 * 100.0;
    println!(
        "  frame-important     {framed} tiles ({:.2}% of the world)",
        share(framed)
    );
    println!(
        "  painted             {painted} tiles ({:.2}%)",
        share(painted)
    );
    println!(
        "  sloped              {sloped} tiles ({:.2}%)",
        share(sloped)
    );
    // A side table keyed by a packed i32 position, holding two i16 frames: roughly 8 bytes of
    // entry plus hashing overhead, which for a `HashMap` runs to about 1.4x that in practice.
    println!(
        "  frames as a side map{:>7.1} MB, against {:.1} MB inline",
        framed as f64 * 12.0 / 1e6,
        tiles as f64 * 4.0 / 1e6
    );
}

fn main() {
    println!("baseline RSS          {:.1} MB\n", rss_mb());

    match env::args().nth(1) {
        Some(path) => {
            let world = wld::load(&PathBuf::from(path)).expect("load world");
            report("loaded world", &world);
            let chests: usize = world
                .chests
                .iter()
                .flatten()
                .map(|c| c.items.len() * size_of::<terrustia_proto::ItemStack>())
                .sum();
            println!("  chest contents      {:.2} MB", chests as f64 / 1e6);
        }
        None => {
            let world = worldgen::generate(4200, 1200, "bench", 1);
            report("generated world", &world);
        }
    }
}
