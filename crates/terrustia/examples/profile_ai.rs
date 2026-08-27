//! Time every NPC routine against a real world, so a slow tick can be blamed on a routine rather
//! than guessed at.
//!
//! ```text
//! cargo run --release --example profile_ai -- "$HOME/.../My.wld"
//! ```

use std::{env, path::PathBuf, process::ExitCode, time::Instant};

use rand::{SeedableRng, rngs::SmallRng};
use terrustia::{
    game::{
        npc::{Npc, TileView},
        npc_ai::{self, AiOutput, Surroundings, Target},
        spawn,
    },
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
        eprintln!("usage: profile_ai <path to .wld>");
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
    let mut rng = SmallRng::seed_from_u64(7);

    // Stand the player on the surface at spawn, where a real one starts.
    let px = f32::from(world.spawn_x) * 16.0;
    let py = f32::from(world.spawn_y) * 16.0;
    let targets = [Target {
        slot: 0,
        center: (px + 10.0, py + 21.0),
        velocity: (0.0, 0.0),
        alive: true,
    }];
    let conditions = terrustia::game::ai::Conditions {
        blood_moon: false,
        day: true,
        eclipse: false,
        raining: false,
        windy: false,
        wind: 0.0,
        desert: false,
        sandstorm: false,
        crimson: world.crimson,
        jungle: false,
        snow: false,
        surface_y: f32::from(world.surface) * 16.0,
        expert: world.game_mode >= 1,
        hardmode: world.progress.hard_mode,
        world_size: (world.width(), world.height()),
    };

    let ticks = 600;
    let mut rows: Vec<(f64, f64, u16, u16)> = Vec::new();
    for npc_type in 1u16..=690 {
        let Some(mut npc) = Npc::new(npc_type, (px + 200.0, py - 40.0), 1) else {
            continue;
        };
        let style = npc.stats.ai_style;
        let mut out = AiOutput::default();
        let mut worst = 0.0f64;
        let started = Instant::now();
        for _ in 0..ticks {
            let began = Instant::now();
            npc_ai::update_with(
                &mut npc,
                &tiles,
                &targets,
                &mut rng,
                &mut out,
                Surroundings {
                    sockets_open: 0,
                    army: Default::default(),
                    treasure: None,
                    mage: Default::default(),
                    conditions,
                    hazards: &[],
                    avoid: &[],
                    target_taken: false,
                    hostile: None,
                    hooks: None,
                    kin_moving: false,
                    census: &[],
                    parent: None,
                    parent_state: 0.0,
                    parent_health: 1.0,
                    slot: 0,
                },
            );
            out.spawn.clear();
            out.shots.clear();
            worst = worst.max(began.elapsed().as_secs_f64() * 1e6);
            if !npc.is_alive() {
                npc.position = (px + 200.0, py - 40.0);
            }
        }
        let mean = started.elapsed().as_secs_f64() * 1e6 / f64::from(ticks);
        rows.push((worst, mean, npc_type, style as u16));
    }
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!("{:>10} {:>10}  type  style  name", "worst us", "mean us");
    for (worst, mean, npc_type, style) in rows.iter().take(25) {
        let name = Npc::new(*npc_type, (0.0, 0.0), 1).map_or("?", |n| n.stats.name);
        println!("{worst:>10.1} {mean:>10.2}  {npc_type:>4}  {style:>5}  {name}");
    }
    let total: f64 = rows.iter().map(|r| r.1).sum();
    println!(
        "\n{} routines, {:.1} us if every one of them ran in the same tick",
        rows.len(),
        total
    );
    let _ = spawn::Biome::Forest;
    ExitCode::SUCCESS
}
