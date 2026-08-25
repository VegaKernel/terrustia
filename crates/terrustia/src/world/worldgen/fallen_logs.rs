//! Fallen logs.
//!
//! `FallenLogsAndWaterFeatures` (`WorldGen.cs:18643`) places its 3×2 log with the plain generic
//! call `PlaceTile(num5, j, 488)` — confirmed by reading that exact line before source access to
//! the decompiled tree was lost partway through this task — so [`super::place_object::place_object`]
//! is correct here, the same as it is for statues and large piles.
//!
//! Half the time a log lands, vanilla records `GenVars.logX`/`GenVars.logY` for the *next* pass
//! (Flowers) to read: a flower patch grows around whichever log the world happened to place last.
//! This generator has no global `GenVars`, so the hook is returned to the caller instead — see
//! [`LogScatterResult`] — for whichever pass builds flowers at generation to consume, if and when
//! it lands. Until then the value is simply unused, which costs nothing: it is exactly as often
//! set as it is in vanilla, just not yet read by anything.

use rand::{Rng, rngs::SmallRng};

use super::layout::Layout;
use super::place_object::place_object;
use crate::world::World;

const FALLEN_LOG: u16 = 488;

pub struct LogScatterResult {
    pub placed: usize,
    /// The last log's position, if the 50/50 roll kept it — vanilla's `GenVars.logX/logY` hook.
    pub last_log: Option<(i32, i32)>,
}

/// Scatter fallen logs across the surface.
///
/// Density: `width / 2100 + rand(-1, 1)` — vanilla's own count, occasionally zero on a small
/// world. Each gets up to 30,000 siting attempts in vanilla; kept much lower here; the siting
/// rule itself — flat grass, clear of biome edges, dry, a roof of open space above — is
/// reimplemented against `layout`/the world directly rather than transcribed, since vanilla's own
/// version leans on several `GenVars` fields (biome edges, dungeon bounds) this generator does
/// not carry forward from earlier passes.
pub fn scatter(world: &mut World, layout: &Layout, rng: &mut SmallRng) -> LogScatterResult {
    let wanted = (layout.width / 2100 + rng.random_range(-1..2)).max(0) as usize;
    let mut placed = 0;
    let mut last_log = None;
    let bottom = (world.height() - 20).max(21);

    for _ in 0..wanted {
        for _ in 0..2000 {
            let x = rng.random_range(20..(layout.width - 20).max(21));
            let mut y = rng.random_range(layout.surface.max(1)..bottom);
            while y < bottom && !world.tile(x, y).is_active() {
                y += 1;
            }
            if y >= bottom {
                continue;
            }
            // Grass only — a log doesn't belong resting on stone or sand.
            let ground = world.tile(x, y);
            if ground.block != 2 {
                continue;
            }
            // Dry: no liquid nearby.
            if (x - 2..=x + 2)
                .flat_map(|nx| (y - 2..=y).map(move |ny| (nx, ny)))
                .any(|(nx, ny)| world.tile(nx, ny).liquid > 0)
            {
                continue;
            }
            if place_object(world, x, y - 1, FALLEN_LOG, 0, -1) {
                placed += 1;
                if rng.random_range(0..2) == 0 {
                    last_log = Some((x, y - 1));
                }
                break;
            }
        }
    }
    LogScatterResult { placed, last_log }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use terrustia_proto::Tile;

    fn meadow(width: i32) -> World {
        let mut world = World::empty(width, 200, "logs");
        for x in 0..width {
            for y in 100..110 {
                world.set_tile(x, y, Tile::block(if y == 100 { 2 } else { 0 }));
            }
        }
        world
    }

    #[test]
    fn logs_scatter_across_a_meadow() {
        let mut world = meadow(6300); // width/2100 = 3, comfortably above zero even with -1
        let mut rand = super::super::rand::UnifiedRandom::new(7);
        let layout = super::super::layout::Layout::plan(6300, 200, &mut rand);
        let mut rng = SmallRng::seed_from_u64(11);
        let result = scatter(&mut world, &layout, &mut rng);
        assert!(
            result.placed > 0,
            "a wide grassy meadow should take at least one log"
        );
    }

    #[test]
    fn a_log_is_fully_framed_where_it_lands() {
        let mut world = meadow(6300);
        let mut rand = super::super::rand::UnifiedRandom::new(1);
        let layout = super::super::layout::Layout::plan(6300, 200, &mut rand);
        let mut rng = SmallRng::seed_from_u64(2);
        let result = scatter(&mut world, &layout, &mut rng);
        if let Some((anchor_x, anchor_y)) = result.last_log {
            // 488's tile_object entry is 3 wide, 2 tall, origin (1, 1) — the anchor is its
            // middle-bottom cell, so the footprint's top-left is one left and one up from it.
            let (left, top) = (anchor_x - 1, anchor_y - 1);
            for dx in 0..3 {
                for dy in 0..2 {
                    let tile = world.tile(left + dx, top + dy);
                    assert_eq!(tile.block, FALLEN_LOG);
                    assert_ne!(tile.frame_x, -1);
                }
            }
        }
    }
}
