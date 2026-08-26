//! Underworld ruins: small ruined ash/hellstone-brick rooms scattered along the underworld
//! surface, plus the real, separate `Hellforges` pass that sites a real Hellforge onto their
//! walls.
//!
//! Transcribed from `AddHellHouses`/`HellFort` (`WorldGen.cs:32431-33416`) and the `Hellforges`
//! generation pass (`WorldGen.cs:18316-18366`).
//!
//! **`Hellforges` is transcribed faithfully in full** — genuinely small (the ~51-line pass this
//! project's own sizing table already measured it at): pick a random underworld point, require it
//! to already sit on `HellstoneBrickUnsafe`/`ObsidianBrickUnsafe` wall (13/14 — exactly the
//! background wall [`scatter_ruins`] leaves behind), scan up to the first solid ground, place a
//! real Hellforge there via [`super::place_object`].
//!
//! **`HellFort` itself is not.** Real vanilla ruins are a procedural 5-column-by-10-row room grid
//! (`array`/`array2`/`array3`/`array4`, a `bool[5,10]` occupancy map, a random walk deciding which
//! cells connect, doors between every adjacent occupied pair, crumbling-wall variance, and — in
//! `AddHellHouses`' own third loop, entirely separate from `HellFort` — a thousand-site budget of
//! 13 hand-authored furniture vignettes dropped near a ruin's walls) — real complexity on the
//! order of this generator's own `pyramids.rs`, not a small item. What's transcribed here is the
//! *site-walking loop* (`AddHellHouses`' first loop: step across the underworld surface at random
//! gaps, find real ground, roll the real material) and a single real hollow room in that
//! material, with one real door — the structural identity a ruin needs to read as one at all.
//! Disclosed and skipped: the 5×10 multi-room grid, the crumbling-wall pass
//! (`HellFort_AttemptToCrumbleWall`), the demon-torch decoration loop (`AddHellHouses`' second
//! loop — a torch near a ruin's wall, not the same thing as a real `Hellforges` placement,
//! confirmed by reading it: it places tile *4*, an ordinary torch, not tile *77*), and the
//! thirteen-vignette treasure catalog (`AddHellHouses`' third loop) — the same shape of
//! furniture-catalog cut `underground_cabins.rs` already disclosed, here scaled to match how much
//! larger vanilla's own version of it is. Every secret-seed branch (`drunkWorldGen`,
//! `remixWorldGen`, `SecretSeed.errorWorld`, `getGoodWorldGen`) is left out, the same standing rule
//! as everywhere else this session.

use super::layout::Layout;
use super::place_object::place_object;
use super::rand::UnifiedRandom;
use crate::world::World;
use terrustia_proto::Tile;

/// `ObsidianBrick`/`HellstoneBrick` (`TileID.cs`) and their matching `*Unsafe` walls
/// (`WallID.cs`) — vanilla's own default pair (`tileType = 75`, `wallType = 14`) four times out of
/// five, the other roll (`tileType = 76`, `wallType = 13`) approximated here as a flat 20% rather
/// than the exact `Next(75, 77)`-then-`Next(5) > 0` double-roll vanilla uses to get there.
const OBSIDIAN_BRICK: u16 = 75;
const OBSIDIAN_BRICK_WALL: u16 = 14;
const HELLSTONE_BRICK: u16 = 76;
const HELLSTONE_BRICK_WALL: u16 = 13;

/// A door style vanilla's own `HellFort` uses for every door it places (`doorStyle = 19`).
const DOOR_STYLE: i32 = 19;

fn material(rand: &mut UnifiedRandom) -> (u16, u16) {
    if rand.next_max(5) == 0 {
        (HELLSTONE_BRICK, HELLSTONE_BRICK_WALL)
    } else {
        (OBSIDIAN_BRICK, OBSIDIAN_BRICK_WALL)
    }
}

/// Real ground under the underworld's ash, found scanning up from just above the floor — the
/// same "walk up past active-or-wet tiles" shape `AddHellHouses`' own site loop uses, so a ruin
/// only ever sites where there is real ash to sit on, not floating over open lava.
fn find_surface(world: &World, x: i32, layout: &Layout) -> Option<i32> {
    let mut y = layout.height - 40;
    while y > layout.underworld {
        let t = world.tile(x, y);
        if !t.is_active() && t.liquid == 0 {
            break;
        }
        y -= 1;
    }
    if y <= layout.underworld {
        return None;
    }
    world.tile(x, y + 1).is_active().then_some(y)
}

/// One ruin: a hollow room, walled in the rolled material, one door.
fn place_ruin(world: &mut World, x: i32, y: i32, rand: &mut UnifiedRandom) {
    let (tile_type, wall_type) = material(rand);
    let width = rand.next_range(9, 16);
    let height = rand.next_range(6, 11);
    let (rx, ry) = (x - width / 2, y - height + 1);

    for dx in 0..width {
        for dy in 0..height {
            let (cx, cy) = (rx + dx, ry + dy);
            if dx == 0 || dx == width - 1 || dy == 0 || dy == height - 1 {
                let mut t = Tile::block(tile_type);
                t.frame_x = -1;
                t.frame_y = -1;
                world.set_tile(cx, cy, t);
            } else {
                // Never written as material to begin with, so there is no stale `block` left on
                // an inactive tile for a save/reload to normalize away — the same fix
                // `underground_cabins.rs::carve_room` needed after being caught by
                // `a_generated_world_survives_a_save`.
                let t = Tile {
                    wall: wall_type,
                    ..Tile::default()
                };
                world.set_tile(cx, cy, t);
            }
        }
    }
    // A door set into the left wall, flush with the floor — `place_object` needs its whole 1x3
    // footprint empty first (the wall carve above just filled it with solid brick) and the row
    // beneath solid (the floor row already is), so the clear has to come before the placement,
    // matching vanilla's own `PlaceDoors` (`ClearTile` then `PlaceTile`).
    let door_y = ry + height - 4;
    for dy in 0..3 {
        let mut t = world.tile(rx, door_y + dy);
        t.flags.set(terrustia_proto::TileFlags::ACTIVE, false);
        world.set_tile(rx, door_y + dy, t);
    }
    place_object(world, rx, door_y, 10, DOOR_STYLE, -1);
}

/// `AddHellHouses`' own site-walking loop: step across the underworld surface at random gaps,
/// siting a real ruin wherever real ground turns up. Returns how many were placed.
pub fn scatter_ruins(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) -> usize {
    let margin = 200;
    if layout.width <= margin * 2 || layout.height <= layout.underworld + 60 {
        return 0;
    }
    let mut placed = 0usize;
    let mut x = margin;
    while x < layout.width - margin {
        if let Some(y) = find_surface(world, x, layout) {
            place_ruin(world, x, y, rand);
            placed += 1;
        }
        x += rand.next_range(30, 130);
    }
    placed
}

/// The real `Hellforges` pass, transcribed in full: find a point already sitting on
/// `HellstoneBrickUnsafe`/`ObsidianBrickUnsafe` wall, scan up to solid ground, place a real
/// Hellforge. Returns how many were placed.
pub fn scatter_hellforges(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) -> usize {
    if layout.width <= 200 || layout.height <= layout.underworld + 50 {
        return 0;
    }
    let wanted = (layout.width / 200).max(1) as usize;
    let mut placed = 0usize;
    for _ in 0..wanted {
        let mut tries = 0;
        loop {
            tries += 1;
            if tries > 10_000 {
                break;
            }
            let x = rand.next_range(1, layout.width - 1);
            let mut y = rand.next_range(layout.height - 250, layout.height - 30);
            if !matches!(
                world.tile(x, y).wall,
                OBSIDIAN_BRICK_WALL | HELLSTONE_BRICK_WALL
            ) {
                continue;
            }
            // Scan down to the first solid (floor) tile, then back up one — landing in the open
            // space directly above the floor, where the forge itself goes. `place_object` does
            // its own "is the row beneath solid" check on that final position, so there is
            // nothing left to validate here beyond finding it.
            while !world.tile(x, y).is_active() && y < layout.height - 20 {
                y += 1;
            }
            y -= 1;
            if place_object(world, x, y, 77, 0, -1) {
                placed += 1;
                break;
            }
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use crate::world::worldgen::rand::UnifiedRandom as Rand;

    /// A synthetic underworld band, just for the small-world guard below — `scatter_ruins`'s own
    /// guard has to trip before either function ever scans a tile, so this never needs to be
    /// realistic the way the two tests below do.
    fn ash_world(width: i32, height: i32, underworld: i32) -> (World, Layout) {
        let world = World::empty(width, height, "underworld-ruins-test");
        let mut rand = Rand::new(1);
        let mut layout = Layout::plan(width, height, &mut rand);
        layout.underworld = underworld;
        (world, layout)
    }

    #[test]
    fn a_small_world_returns_zero_rather_than_panicking() {
        let (mut world, layout) = ash_world(300, 200, 100);
        let mut rand = Rand::new(1);
        assert_eq!(scatter_ruins(&mut world, &layout, &mut rand), 0);
        assert_eq!(scatter_hellforges(&mut world, &layout, &mut rand), 0);
    }

    /// Against a real generated world rather than a hand-built fixture: `structures::underworld`
    /// only *hollows* an already-solid ash fill (`terrain::fill`'s job, much earlier in the real
    /// pipeline) — built standalone, it hollows nothing but empty air, and `find_surface` never
    /// finds a real transition. The same lesson `underground_cabins.rs` already needed once for
    /// exactly this reason.
    #[test]
    fn ruins_carve_real_hollow_rooms_with_a_door() {
        let (world, built) = super::super::build(2200, 900, "underworld-ruins-test", 7);
        assert!(
            built.underworld_ruins > 0,
            "a real generated world should take at least one ruin"
        );

        let mut brick_tiles = 0;
        let mut doors = 0;
        for x in 0..world.width() {
            for y in 0..world.height() {
                let t = world.tile(x, y);
                if t.is_active() && matches!(t.block, OBSIDIAN_BRICK | HELLSTONE_BRICK) {
                    brick_tiles += 1;
                }
                if t.is_active() && t.block == 10 {
                    doors += 1;
                }
            }
        }
        assert!(
            brick_tiles > 20,
            "expected real carved ruin walls, got {brick_tiles} brick tiles"
        );
        assert!(doors > 0, "expected at least one real door placed");
    }

    #[test]
    fn hellforges_only_site_on_a_ruins_own_wall() {
        let (world, built) = super::super::build(2200, 900, "underworld-hellforges-test", 7);
        assert!(
            built.hellforges > 0,
            "hellforges should site against the ruins a real generated world already has"
        );

        let mut forges = 0;
        for x in 0..world.width() {
            for y in 0..world.height() {
                if world.tile(x, y).is_active() && world.tile(x, y).block == 77 {
                    forges += 1;
                }
            }
        }
        assert!(forges > 0, "expected at least one real Hellforge tile");
    }

    /// Real placement counts on real generated worlds — not asserted, just printed. Run with
    /// `cargo test -p terrustia --lib underworld_ruins::tests::measure_on_real_worlds --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn measure_on_real_worlds() {
        for seed in [999u64, 4242, 12345] {
            let (_world, built) = super::super::build(4200, 1200, "measure", seed);
            eprintln!(
                "seed {seed}: underworld_ruins={} hellforges={}",
                built.underworld_ruins, built.hellforges
            );
        }
    }
}
