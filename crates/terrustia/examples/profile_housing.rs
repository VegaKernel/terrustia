//! Time the housing scan the server runs every five seconds, against a real world.
//!
//! ```text
//! cargo run --release --example profile_housing -- "$HOME/.../My.wld"
//! ```

use std::{env, path::PathBuf, process::ExitCode, time::Instant};

use terrustia::{game::housing, world::wld};

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: profile_housing <path to .wld>");
        return ExitCode::FAILURE;
    };
    let world = match wld::load(&path) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    // The same probe grid `GameServer::find_free_house` walks, centred on a few places a player
    // realistically stands.
    let spawn = (i32::from(world.spawn_x), i32::from(world.spawn_y));
    let spots = [
        ("spawn", spawn),
        ("underground", (spawn.0, i32::from(world.surface) + 60)),
        ("cavern", (spawn.0, i32::from(world.rock_layer) + 60)),
        ("sky", (spawn.0, 40)),
    ];

    for (label, (px, py)) in spots {
        let started = Instant::now();
        let mut probes = 0u32;
        let mut found = 0u32;
        for dx in (-60..=60).step_by(5) {
            for dy in (-40..=40).step_by(5) {
                probes += 1;
                if housing::check_room(&world, px + dx, py + dy).is_ok() {
                    found += 1;
                }
            }
        }
        let took = started.elapsed();
        println!(
            "{label:12} at ({px},{py}): {probes} probes, {found} houses, {:>8.3} ms",
            took.as_secs_f64() * 1000.0
        );
    }
    ExitCode::SUCCESS
}
