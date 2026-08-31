//! What the liquid read inside `step_physics` costs, against a real world.
//!
//! `UpdateNPC_UpdateGravity` picks its gravity and terminal speed from whatever the NPC is standing
//! in, so the physics has to read one tile per NPC per tick that it did not read before. This is
//! the number behind the claim that the read is free at any NPC count a real server reaches: it
//! times the read on its own, then the whole step it sits inside, then converts to a share of the
//! 16.67 ms frame. Run it on an otherwise idle machine, or the load lands in the numbers.
//!
//! ```text
//! cargo run --release --example liquid_gravity_cost -- "$HOME/.../My.wld"
//! ```

use std::{env, hint::black_box, path::PathBuf, process::ExitCode, time::Instant};

use terrustia::{
    game::npc::{Npc, TileView, liquid_at, step_physics},
    world::{World, wld},
};
use terrustia_proto::tile::Tile;

struct WorldTiles<'a>(&'a World);
impl TileView for WorldTiles<'_> {
    fn tile(&self, x: i32, y: i32) -> Tile {
        self.0.tile(x, y)
    }
}

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1).map(PathBuf::from) else {
        eprintln!("usage: liquid_gravity_cost <path to .wld>");
        return ExitCode::FAILURE;
    };
    let world = match wld::load(&path) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let tiles = WorldTiles(&world);
    let px = f32::from(world.spawn_x) * 16.0;
    let py = f32::from(world.spawn_y) * 16.0;

    const ROUNDS: u32 = 2_000_000;

    // The added work on its own.
    let mut best_read = f64::MAX;
    for _ in 0..5 {
        let started = Instant::now();
        for i in 0..ROUNDS {
            let at = (px + (i % 64) as f32, py + (i % 32) as f32);
            black_box(liquid_at(black_box(&tiles), black_box(at)));
        }
        best_read = best_read.min(started.elapsed().as_secs_f64() * 1e9 / f64::from(ROUNDS));
    }

    // The whole step it sits inside, for scale.
    let mut best_step = f64::MAX;
    for _ in 0..5 {
        let mut npc = Npc::new(3, (px + 200.0, py - 40.0), 1).expect("zombie");
        let started = Instant::now();
        for _ in 0..ROUNDS {
            step_physics(black_box(&mut npc), black_box(&tiles));
            npc.position = (px + 200.0, py - 40.0);
            npc.velocity = (0.0, 0.0);
        }
        best_step = best_step.min(started.elapsed().as_secs_f64() * 1e9 / f64::from(ROUNDS));
    }

    println!("liquid_at:    {best_read:.2} ns/call");
    println!("step_physics: {best_step:.2} ns/call (the read included)");
    for npcs in [200usize, 1000] {
        println!(
            "{npcs} NPCs a tick: {:.1} us added, {:.4}% of the 16.67 ms budget",
            best_read * npcs as f64 / 1000.0,
            best_read * npcs as f64 / 1e9 / 0.01667 * 100.0
        );
    }
    ExitCode::SUCCESS
}
