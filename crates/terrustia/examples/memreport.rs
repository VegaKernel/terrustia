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
    let bytes = tiles * size_of::<Tile>();
    println!("{label}");
    println!("  size_of::<Tile>()   {} bytes", size_of::<Tile>());
    println!(
        "  tiles               {tiles} ({}x{})",
        world.width(),
        world.height()
    );
    println!("  tile array          {:.1} MB", bytes as f64 / 1e6);
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
