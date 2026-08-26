//! Strips background wall from the surface crust above open cave space, so a freshly-dug entrance
//! isn't walled shut the moment a player breaks through to it.
//!
//! Transcribed from the `DirtWallCleanup` generation pass (`WorldGen.cs:15322-15437`, 116 lines) —
//! two mirrored column scans, left-to-right then right-to-left, each walking down from the top of
//! the world to `worldSurface`. A small state machine per column tracks whether the scan is
//! currently "inside a surface crack" (`flag`): it enters that state on finding five stacked
//! wall-free, inactive tiles in a 2-wide band, and leaves it on hitting solid, non-sand ground.
//! While inside the state, every wall-typed tile the scan crosses — Dirt, Snow, Jungle or Hive
//! unsafe wall — is stripped, and so are up to three tiles to either side, each of the outer two
//! rolled at 50% so the cleared edge doesn't read as a perfectly straight line.
//!
//! **Two real, disclosed vanilla asymmetries, transcribed rather than "fixed" for symmetry** — this
//! project's standing rule is to keep a real vanilla quirk once found, not silently correct it:
//!
//! 1. The self-tile wall clear checks all four wall types (`2, 40, 64, 86` — Dirt/Snow/Jungle/Hive
//!    unsafe), but the ±1/±2/±3 neighbour clears in the *same* loop check `2, 40, 40` — the third
//!    term is a literal duplicate of the second rather than `64` or `86`, so a neighbouring Jungle
//!    or Hive wall is never stripped by the side reach, only a Dirt or Snow one. Confirmed by
//!    reading the decompiled source directly, not inferred.
//! 2. The first (left-to-right) loop's "don't touch sand walls" exclusion covers three types — Sand
//!    (53), Ebonsand (112), Crimsand (234) — and its self-tile clear covers all four wall types. The
//!    *second* (right-to-left) loop's exclusion covers only Sand, and its self-tile clear omits Hive
//!    (86). The two loops are not mirror images of each other in vanilla; they are transcribed here
//!    exactly as asymmetric as they are in source.
//!
//! No siting/measurement dependency on `StructureMap` and no small-world guard needed: every
//! `genRand` draw here is a fixed `Next(2)` coin flip, never a world-size-derived range.

use super::layout::Layout;
use super::rand::UnifiedRandom;
use super::tiles::walls;
use crate::world::World;

/// The four wall types the *self*-tile clear checks in both loops (loop 2 further narrows this —
/// see the module doc's second disclosed asymmetry).
const SELF_WALLS_FULL: [u16; 4] = [walls::DIRT, walls::SNOW, walls::JUNGLE, walls::HIVE];
/// The three wall types the second loop's self-tile clear checks — Hive omitted, faithfully.
const SELF_WALLS_NO_HIVE: [u16; 3] = [walls::DIRT, walls::SNOW, walls::JUNGLE];
/// The two wall types every neighbour reach checks, in *both* loops — `2, 40, 40` in source, so the
/// third slot is a duplicate of the second rather than a distinct type. See the module doc's first
/// disclosed asymmetry.
const NEIGHBOUR_WALLS: [u16; 2] = [walls::DIRT, walls::SNOW];

const SAND_GROUP: [u16; 3] = [
    super::tiles::SAND,
    super::tiles::EBONSAND,
    super::tiles::CRIMSAND,
];

fn clear_wall_if(world: &mut World, x: i32, y: i32, set: &[u16]) {
    let t = world.tile(x, y);
    if set.contains(&t.wall) {
        let mut t = t;
        t.wall = 0;
        world.set_tile(x, y, t);
    }
}

/// One column of one loop direction. `sand_exclusion` is the "don't touch a sand-family wall" set —
/// three types on the first loop, one on the second (the module doc's second asymmetry).
#[allow(clippy::too_many_arguments)]
fn scan_column(
    world: &mut World,
    rand: &mut UnifiedRandom,
    x: i32,
    surface: i32,
    self_walls: &[u16],
    sand_exclusion: &[u16],
) {
    let mut flag = true;
    let mut y = 0;
    while y < surface {
        if flag {
            clear_wall_if(world, x, y, self_walls);

            let here = world.tile(x, y);
            let is_sand_family = here.is_active() && sand_exclusion.contains(&here.block);
            if !here.is_active() || !is_sand_family {
                clear_wall_if(world, x - 1, y, &NEIGHBOUR_WALLS);
                if rand.next_max(2) == 0 {
                    clear_wall_if(world, x - 2, y, &NEIGHBOUR_WALLS);
                }
                if rand.next_max(2) == 0 {
                    clear_wall_if(world, x - 3, y, &NEIGHBOUR_WALLS);
                }
                clear_wall_if(world, x + 1, y, &NEIGHBOUR_WALLS);
                if rand.next_max(2) == 0 {
                    clear_wall_if(world, x + 2, y, &NEIGHBOUR_WALLS);
                }
                if rand.next_max(2) == 0 {
                    clear_wall_if(world, x + 3, y, &NEIGHBOUR_WALLS);
                }
                if world.tile(x, y).is_active() {
                    flag = false;
                }
            }
        } else {
            let open = (0..5).all(|dy| world.tile(x, y + dy).wall == 0)
                && world.tile(x - 1, y).wall == 0
                && world.tile(x + 1, y).wall == 0
                && world.tile(x - 2, y).wall == 0
                && world.tile(x + 2, y).wall == 0
                && (0..4).all(|dy| !world.tile(x, y + dy).is_active());
            if open {
                flag = true;
            }
        }
        y += 1;
    }
}

/// The `DirtWallCleanup` pass. Returns how many wall cells were cleared, across both scans — a
/// cleanup pass, so a tile count is the meaningful measurement, not a placement count.
pub fn scrub(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) -> usize {
    let width = world.width();
    let total_before = count_all_walls(world);

    for x in 3..width - 3 {
        scan_column(
            world,
            rand,
            x,
            layout.surface,
            &SELF_WALLS_FULL,
            &SAND_GROUP,
        );
    }
    for x in (5..width - 5).rev() {
        scan_column(
            world,
            rand,
            x,
            layout.surface,
            &SELF_WALLS_NO_HIVE,
            &[super::tiles::SAND],
        );
    }

    total_before.saturating_sub(count_all_walls(world))
}

fn count_all_walls(world: &World) -> usize {
    let mut n = 0;
    for x in 0..world.width() {
        for y in 0..world.height() {
            if world.tile(x, y).wall != 0 {
                n += 1;
            }
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::worldgen::layout::Layout;
    use terrustia_proto::Tile;

    fn plan(width: i32, height: i32, seed: i32) -> Layout {
        let mut rand = UnifiedRandom::new(seed);
        Layout::plan(width, height, &mut rand)
    }

    #[test]
    fn a_wall_over_an_open_surface_crack_is_stripped() {
        let mut world = World::empty(200, 200, "dirt-wall-cleanup");
        let layout = plan(200, 200, 11);
        // An open shaft from the top of the world down through the surface, walled with dirt wall
        // the whole way, at x=100.
        for y in 0..layout.surface {
            let mut t = Tile::AIR;
            t.wall = walls::DIRT;
            world.set_tile(100, y, t);
            world.set_tile(99, y, t);
            world.set_tile(101, y, t);
        }
        assert!(world.tile(100, 10).wall == walls::DIRT);

        let mut rand = UnifiedRandom::new(11);
        let cleared = scrub(&mut world, &layout, &mut rand);

        assert!(cleared > 0, "expected some wall cleared over an open shaft");
        assert_eq!(
            world.tile(100, 10).wall,
            0,
            "wall directly over the open shaft should be cleared"
        );
    }

    #[test]
    fn solid_ground_with_no_open_crack_keeps_its_wall() {
        let mut world = World::empty(200, 200, "dirt-wall-cleanup-solid");
        let layout = plan(200, 200, 11);
        for x in 90..110 {
            for y in 0..layout.surface {
                let mut t = Tile::block(super::super::tiles::STONE);
                t.wall = walls::DIRT;
                world.set_tile(x, y, t);
            }
        }
        let mut rand = UnifiedRandom::new(11);
        // Solid ground the whole way down: `flag` starts true but the very first row's neighbour
        // clear already sees `tile.active()` and flips `flag` false immediately, and the "reopen"
        // window can never see 5 stacked wall-free tiles since everything here is walled and solid
        // — so this column's wall should survive untouched below the first row or two.
        let cleared = scrub(&mut world, &layout, &mut rand);
        assert!(
            world.tile(100, layout.surface - 1).wall == walls::DIRT,
            "deep solid ground far from any open crack should keep its wall"
        );
        let _ = cleared;
    }

    #[test]
    fn a_tiny_world_does_not_panic() {
        // Uses the 400x300 floor real tests elsewhere in this crate generate full worlds at
        // (`wld_save.rs`, `world.rs`) — anything much smaller trips `Layout::plan`'s own unrelated
        // minimum-band-width assumptions, which is not a defect in this pass to guard against.
        let mut world = World::empty(400, 300, "dirt-wall-cleanup-tiny");
        let layout = plan(400, 300, 1);
        let mut rand = UnifiedRandom::new(1);
        // Every `rand` draw here is a fixed `next_max(2)` — no world-size-derived `next_range` call
        // exists in this pass, so nothing here can panic on a tiny synthetic world.
        let _ = scrub(&mut world, &layout, &mut rand);
    }
}
