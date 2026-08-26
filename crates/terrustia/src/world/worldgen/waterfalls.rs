//! Waterfalls: pound a solid tile into a half-brick wherever liquid presses against one open side
//! of it, so the boundary reads as a lip instead of a sharp step.
//!
//! Transcribed from the `Waterfalls` generation pass (`WorldGen.cs:16712-16770`, 59 lines) —
//! Tier 3's smallest item. Two independent full-world scans:
//!
//! 1. A solid tile with open air to one side and solid ground below gets pounded if liquid is
//!    pressing against either horizontal neighbour, unless a half-bricked tile already sits
//!    somewhere in a randomised vertical band around it (vanilla's own "don't stack two lips close
//!    together" guard) or — rarely, 9 times out of 10 — the tile itself is Bone or Slime.
//! 2. A solid tile with solid ground both below *and* to one side gets pounded if the *other* side
//!    is open, the tile just past that open side is itself half-bricked, and liquid presses against
//!    the far side of that half-brick — the "smooth the corner where an existing waterfall lip
//!    meets a ledge" shape.
//!
//! Reuses [`super::smooth::pound_tile`]/[`super::smooth::solid_tile`] rather than re-deriving
//! `PoundTile`/`SolidTile` — this pass calls exactly the same two vanilla functions `smooth.rs`
//! already ported, generation-time behaviour only (no sound/dust/network side effects), which is
//! exactly what a worldgen-time port needs here too.
//!
//! **One deliberate deviation, matching a scoped toggle vanilla itself uses.** Vanilla flips
//! `Main.tileSolid[191]` (Living Wood) to `false` for the duration of this pass only, so a waterfall
//! never forms against a living tree's trunk, then restores it before the next pass runs. This
//! project's `tile_solid::solid` table has no equivalent scoped-override mechanism, so
//! [`solid_here`] below wraps [`super::smooth::solid_tile`] with the same one-tile exception
//! directly rather than mutating table state every other (single-threaded, sequential) pass would
//! also observe for the duration of this one.
//!
//! No siting/measurement dependency on `StructureMap` — this pass never places a discrete
//! structure, only reshapes tiles that are already there, the same as `smooth.rs` itself.

use terrustia_proto::TileFlags;

use super::layout::Layout;
use super::rand::UnifiedRandom;
use super::smooth::{pound_tile, solid_tile};
use crate::world::World;

/// `TileID.LivingWood` — see the module doc's note on vanilla's scoped `tileSolid` toggle.
const LIVING_WOOD: u16 = 191;
/// `TileID.Bone`/`TileID.SlimeBlock` — the pass's own rare exception in its first scan.
const BONE: u16 = 75;
const SLIME_BLOCK: u16 = 76;

/// `SolidTile`, with Living Wood forced non-solid for the duration of this pass — see the module
/// doc.
fn solid_here(world: &World, x: i32, y: i32) -> bool {
    if world.tile(x, y).block == LIVING_WOOD {
        return false;
    }
    solid_tile(world, x, y)
}

fn has_liquid(world: &World, x: i32, y: i32) -> bool {
    world.tile(x, y).liquid > 0
}

/// The `Waterfalls` pass. Returns how many tiles were pounded into a half-brick, across both
/// scans — a cleanup/reshaping pass, so a tile count is the meaningful measurement, not a placement
/// count.
pub fn scatter(world: &mut World, _layout: &Layout, rand: &mut UnifiedRandom) -> usize {
    let (width, height) = (world.width(), world.height());
    let mut pounded = 0usize;

    // First scan: an ordinary "water pours off this ledge" lip.
    for x in 20..width - 20 {
        for y in 20..height - 20 {
            if !(solid_here(world, x, y)
                && !world.tile(x - 1, y).is_active()
                && solid_here(world, x, y + 1)
                && !world.tile(x + 1, y).is_active()
                && (has_liquid(world, x - 1, y) || has_liquid(world, x + 1, y)))
            {
                continue;
            }

            let mut ok = true;
            let above = rand.next_range(8, 20);
            let below = rand.next_range(8, 20);
            let (top, bottom) = (y - above, y + below);
            for k in top..=bottom {
                let t = world.tile(x, k);
                if t.is_active() && t.flags.has(TileFlags::HALF_BRICK) {
                    ok = false;
                }
            }
            let block = world.tile(x, y).block;
            if (block == BONE || block == SLIME_BLOCK) && rand.next_max(10) != 0 {
                ok = false;
            }
            if ok && pound_tile(world, x, y) {
                pounded += 1;
            }
        }
    }

    // Second scan: smooth the corner where an existing lip meets a ledge.
    for x in 20..width - 20 {
        for y in 20..height - 20 {
            let t = world.tile(x, y);
            // `TileID.Spikes`/`WoodenSpikes` — never touched by this pass, matching vanilla's own
            // exclusion (a spike trap's frame-important half-brick bit means something else).
            if t.block == super::tiles::SPIKES || t.block == 232 {
                continue;
            }
            if !(solid_here(world, x, y) && solid_here(world, x, y + 1)) {
                continue;
            }

            let left = world.tile(x - 1, y);
            let right = world.tile(x + 1, y);
            if !solid_here(world, x + 1, y)
                && left.is_active()
                && left.flags.has(TileFlags::HALF_BRICK)
                && has_liquid(world, x - 2, y)
                && pound_tile(world, x, y)
            {
                pounded += 1;
                continue;
            }
            if !solid_here(world, x - 1, y)
                && right.is_active()
                && right.flags.has(TileFlags::HALF_BRICK)
                && has_liquid(world, x + 2, y)
                && pound_tile(world, x, y)
            {
                pounded += 1;
            }
        }
    }

    pounded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::worldgen::layout::Layout;
    use terrustia_proto::{Liquid, Tile};

    fn small_world() -> World {
        World::empty(200, 200, "waterfalls")
    }

    #[test]
    fn a_solid_ledge_with_water_pressing_against_one_side_gets_pounded() {
        let mut world = small_world();
        for x in 0..200 {
            for y in 0..200 {
                world.set_tile(x, y, Tile::block(super::super::tiles::STONE));
            }
        }
        // Clear a small alcove: (50, 50) is the solid ledge, (49, 50) is open water, (51, 50) is
        // open air, (50, 51) is solid ground beneath the ledge.
        world.set_tile(49, 50, Tile::AIR.with_liquid(Liquid::Water, 200));
        world.set_tile(51, 50, Tile::AIR);
        // Clear a generous band around (50, 50) so no pre-existing half-brick blocks the roll.
        for y in 30..70 {
            let mut t = world.tile(50, y);
            t.flags.set(TileFlags::HALF_BRICK, false);
            world.set_tile(50, y, t);
        }

        let mut rand = UnifiedRandom::new(7);
        let plan = Layout::plan(200, 200, &mut rand);
        let mut rand = UnifiedRandom::new(7);
        let pounded = scatter(&mut world, &plan, &mut rand);

        assert!(pounded >= 1, "expected at least one pounded ledge");
        let ledge = world.tile(50, 50);
        assert!(
            ledge.flags.has(TileFlags::HALF_BRICK),
            "the ledge should have been pounded into a half-brick"
        );
    }

    #[test]
    fn a_tiny_world_does_not_panic() {
        // Every bound here is a fixed literal (8..20) or the plain `20..width-20`/`20..height-20`
        // loop ranges, which produce an empty iterator rather than panicking when the world is
        // smaller than 40 tiles on a side — no `next_range` call is ever fed a world-derived span.
        // Uses the 400x300 floor real tests elsewhere in this crate generate full worlds at
        // (`wld_save.rs`, `world.rs`) rather than something smaller, since anything much smaller
        // trips `Layout::plan`'s own unrelated minimum-band-width assumptions.
        let mut world = World::empty(400, 300, "tiny-waterfalls");
        let mut rand = UnifiedRandom::new(1);
        let plan = Layout::plan(400, 300, &mut rand);
        let mut rand = UnifiedRandom::new(1);
        let pounded = scatter(&mut world, &plan, &mut rand);
        assert_eq!(pounded, 0);
    }
}
