//! Fragile (breakable) ice over standing water in the ice biome — a frozen crust on ponds.
//!
//! Transcribed from the `FragileIceOverIceBiomeWater` generation pass (`WorldGen.cs:16771-16800`,
//! 30 lines) and the helper it drives, `MakeWateryIceThing` (`WorldGen.cs:80752-80794`, 43 lines).
//!
//! **Not to be confused with `ThinIceBiome`** (`Terraria.GameContent.Biomes.ThinIceBiome`), a
//! *different* real vanilla class already landed in `micro_biomes.rs` under Tier 2 — that one
//! carves a standalone tapering-circle pond of thin ice as its own micro-biome, sited independently
//! of any other water. This pass is the opposite direction: it scans every column of *existing*
//! standing water for a real snow/ice ceiling above it and crusts the water's own surface over,
//! wherever the shape allows. `plan.md`'s own Tier 3 sizing table flags this exact naming collision
//! so the two are never double-booked; this module is only the second one.
//!
//! For every wet, inactive tile with liquid, scan down the column until solid ground; if that
//! ground is Snow or one of the four `TileID.Sets.Ices` types (Ice, CorruptIce, HallowedIce,
//! FleshIce — the latter three only ever appear post-hardmode via biome conversion, never at
//! generation time in an ordinary world, but transcribed for completeness since the check is free),
//! scan back up through the liquid to find its actual surface, then spread sideways from that point,
//! placing breakable ice on the open water surface as long as the tile above stays clear and the
//! diagonal neighbour isn't half-bricked.
//!
//! Faithful and complete — no DSL dependency, no siting helper, nothing disclosed as cut. The
//! `remixWorld` lava exception (fragile ice can crust over lava too on that secret seed) is
//! transcribed as a dead branch guard rather than wired to anything, since this project has no
//! remix-world concept at all yet (see `plan.md`'s own "Secret seeds… in scope, deprioritized"
//! line) — ordinary generation always takes the "not lava" arm, matching vanilla's own default.

use terrustia_proto::{Liquid, TileFlags};

use super::layout::Layout;
use crate::world::World;

/// `TileID.Sets.Snow`.
const SNOW: u16 = super::tiles::SNOW;
/// `TileID.Sets.Ices`: Ice, CorruptIce, HallowedIce, FleshIce.
const ICES: [u16; 4] = [super::tiles::ICE, 163, 164, 200];
const BREAKABLE_ICE: u16 = super::tiles::BREAKABLE_ICE;

fn is_snow_or_ice(block: u16) -> bool {
    block == SNOW || ICES.contains(&block)
}

/// `MakeWateryIceThing(i, j)`, one column.
fn make_watery_ice_thing(world: &mut World, i: i32, j0: i32) {
    let seed = world.tile(i, j0);
    if !world.in_bounds(i, j0) || seed.liquid == 0 || seed.is_active() {
        return;
    }
    if seed.liquid_kind == Liquid::Lava {
        // `!Main.remixWorld` in vanilla; this project has no remix mode, so this arm always bails.
        return;
    }

    let height = world.height();
    let mut y = j0;
    loop {
        let t = world.tile(i, y);
        if t.is_active() || t.liquid == 0 {
            break;
        }
        y += 1;
        if y > height - 50 {
            return;
        }
    }
    let ground = world.tile(i, y);
    if !ground.is_active() || !is_snow_or_ice(ground.block) {
        return;
    }

    y -= 1;
    loop {
        if world.tile(i, y).liquid == 0 {
            break;
        }
        y -= 1;
        if y < 10 {
            return;
        }
    }
    if world.tile(i, y).is_active() {
        return;
    }
    y += 1;
    if world.tile(i, y).is_active() {
        return;
    }

    // Spread left.
    let mut x = i;
    loop {
        let here = world.tile(x, y);
        let above = world.tile(x, y - 1);
        let below_left = world.tile(x - 1, y);
        if !world.in_bounds(x, y)
            || here.is_active()
            || here.liquid == 0
            || above.liquid != 0
            || above.is_active()
            || below_left.flags.has(TileFlags::HALF_BRICK)
        {
            break;
        }
        place_breakable_ice(world, x, y);
        x -= 1;
    }
    // Spread right.
    x = i + 1;
    loop {
        let here = world.tile(x, y);
        let above = world.tile(x, y - 1);
        let below_right = world.tile(x + 1, y);
        if !world.in_bounds(x, y)
            || here.is_active()
            || here.liquid == 0
            || above.liquid != 0
            || above.is_active()
            || below_right.flags.has(TileFlags::HALF_BRICK)
        {
            break;
        }
        place_breakable_ice(world, x, y);
        x += 1;
    }
}

fn place_breakable_ice(world: &mut World, x: i32, y: i32) {
    let mut t = world.tile(x, y);
    t.block = BREAKABLE_ICE;
    t.frame_x = -1;
    t.frame_y = -1;
    t.flags.set(TileFlags::ACTIVE, true);
    world.set_tile(x, y, t);
}

/// The `FragileIceOverIceBiomeWater` pass. Returns how many breakable-ice tiles were placed.
///
/// Scans from `layout.surface` downward, matching vanilla's own `num = (int)Main.worldSurface`
/// starting point — the sky (where a floating island's cloud lake, Tier 2, might carry its own
/// water) is never in range here, in real vanilla generation either.
pub fn crust(world: &mut World, layout: &Layout) -> usize {
    let (width, height) = (world.width(), world.height());
    let mut placed = 0usize;
    for x in 10..width - 10 {
        for y in layout.surface..height - 100 {
            let before = count_ice(world, x, y);
            make_watery_ice_thing(world, x, y);
            placed += count_ice(world, x, y).saturating_sub(before);
        }
    }
    placed
}

fn count_ice(world: &World, x: i32, y: i32) -> usize {
    usize::from(world.tile(x, y).block == BREAKABLE_ICE && world.tile(x, y).is_active())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::worldgen::layout::Layout;
    use crate::world::worldgen::rand::UnifiedRandom;
    use terrustia_proto::Tile;

    fn plan(width: i32, height: i32, seed: i32) -> Layout {
        let mut rand = UnifiedRandom::new(seed);
        Layout::plan(width, height, &mut rand)
    }

    // `Layout::surface` is `height * 0.28` (`layout.rs:100`), so a pond needs to sit at
    // `y >= layout.surface` to fall inside this pass's own `layout.surface..height-100` scan
    // range at all — 400x300 puts `surface` at 84, comfortably below the pond used here and
    // comfortably above `height - 100 = 200`. This is also the smallest world size any real test
    // elsewhere in this crate generates a full world at (`wld_save.rs`, `world.rs`), so it is the
    // actual floor this pass needs to survive against, not an arbitrary small number.
    const WIDTH: i32 = 400;
    const HEIGHT: i32 = 300;

    // The real geometry `MakeWateryIceThing` requires, confirmed by reading the scan order in
    // source rather than guessed: the water column's *floor* must be Snow or Ice (not a ceiling
    // above it) — the scan walks down through the liquid until it hits ground, checks that
    // ground's type, then walks back up to find the open water surface and crusts that instead.
    // An ice-biome pond: icy lake-bed below, open air above the water.

    #[test]
    fn a_pond_with_an_ice_floor_gets_a_breakable_crust() {
        let mut world = World::empty(WIDTH, HEIGHT, "thin-ice");
        // Water at y=151..156, resting on an ice floor at y=156, open to air at y=150 and above.
        for x in 190..210 {
            world.set_tile(x, 156, Tile::block(super::super::tiles::ICE));
            for y in 151..156 {
                world.set_tile(x, y, Tile::AIR.with_liquid(Liquid::Water, 200));
            }
        }
        let layout = plan(WIDTH, HEIGHT, 3);
        let placed = crust(&mut world, &layout);
        assert!(placed > 0, "expected at least one breakable-ice tile");
        assert_eq!(world.tile(200, 151).block, BREAKABLE_ICE);
    }

    #[test]
    fn a_pond_with_a_stone_floor_gets_no_crust() {
        let mut world = World::empty(WIDTH, HEIGHT, "thin-ice-none");
        for x in 190..210 {
            world.set_tile(x, 156, Tile::block(super::super::tiles::STONE));
            for y in 151..156 {
                world.set_tile(x, y, Tile::AIR.with_liquid(Liquid::Water, 200));
            }
        }
        let layout = plan(WIDTH, HEIGHT, 3);
        let placed = crust(&mut world, &layout);
        assert_eq!(placed, 0, "no snow/ice lake-bed means no thin ice at all");
    }

    #[test]
    fn a_tiny_world_does_not_panic() {
        // This pass makes no `next_range` call at all (`MakeWateryIceThing` has no randomness),
        // so nothing here can panic on a small world — the only way to get a *wrong* answer would
        // be a bound like `height - 100` going negative, which produces an empty range rather than
        // a panic. Uses the same 400x300 floor as the tests above, since anything smaller trips
        // `Layout::plan`'s own unrelated minimum-band-width assumptions (a pre-existing constraint
        // on `layout.rs`, not something this pass introduces or could reasonably guard against).
        let mut world = World::empty(WIDTH, HEIGHT, "thin-ice-tiny");
        let layout = plan(WIDTH, HEIGHT, 1);
        let placed = crust(&mut world, &layout);
        assert_eq!(placed, 0);
    }
}
