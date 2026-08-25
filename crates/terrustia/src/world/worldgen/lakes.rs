//! Surface lakes.
//!
//! A generated world had water in exactly two places: the two oceans, and the lava in the
//! underworld. Between them, four thousand tiles of dry land with not a pond on it. Water on the
//! surface is not decoration — it is where the first bucket comes from, what a Water Candle is
//! found near, and half of what makes a landscape read as one.
//!
//! **Reimplemented rather than transcribed, deliberately.** Vanilla's `Lakes` pass sites a lake by
//! rejecting positions near `GenVars.mCaveX`, `GenVars.tunnelX` and
//! `GenVars.UndergroundDesertLocation` — state written by earlier passes this generator does not
//! have. Copying that filter faithfully would be copying a test that can never fire, which is
//! worse than useless: it would look like parity and behave like nothing. So the shape is ours,
//! and the rules it enforces are the ones that actually matter — a lake needs level ground, needs
//! somewhere for the water to sit, and must not be cut open by a cave underneath it.

use super::rand::UnifiedRandom;
use terrustia_proto::{Liquid, Tile};

use super::layout::{Layout, Surface};
use super::tiles;
use crate::world::World;

/// How wide a lake may be, in tiles.
///
/// Small enough to sit in the dips a heightmap naturally makes, rather than needing a valley the
/// terrain does not have.
const MIN_WIDTH: i32 = 14;
const MAX_WIDTH: i32 = 46;

/// How deep, at the middle. A lake is a saucer, not a well.
const MAX_DEPTH: i32 = 9;

/// How level the ground has to be across a lake's width before one will sit there.
///
/// The whole difficulty of putting a lake on a heightmap: on a slope the water runs out of the low
/// side, and there is no run-time settling at generation to notice. Requiring flat ground is what
/// replaces that.
const MAX_SLOPE: i32 = 3;

/// Carve lakes into the surface and fill them.
///
/// Returns how many were made.
pub fn carve(
    world: &mut World,
    layout: &Layout,
    heights: &[i32],
    rand: &mut UnifiedRandom,
) -> usize {
    // Vanilla scales its lake count with world width; so does this.
    let wanted = (layout.width / 900).max(2) as usize;
    let mut made = 0usize;
    // Bounded independently of success, so a world with nowhere suitable finishes rather than
    // spinning. Most attempts fail, which is expected — the ground is rarely level enough.
    let attempts = layout.width / 3;

    for _ in 0..attempts {
        if made >= wanted {
            break;
        }
        let width = rand.next_range(MIN_WIDTH, MAX_WIDTH);
        let x = rand.next_range(width + 4, layout.width - width - 4);
        if let Some(()) = try_lake(world, layout, heights, x, width, rand) {
            made += 1;
        }
    }
    made
}

/// Put one lake at `x`, if the ground there will hold it.
fn try_lake(
    world: &mut World,
    layout: &Layout,
    heights: &[i32],
    x: i32,
    width: i32,
    rand: &mut UnifiedRandom,
) -> Option<()> {
    let half = width / 2;

    // Not in an ocean — there is already water there — and not in the desert, where vanilla puts
    // an oasis instead and a plain pond looks wrong.
    for probe in [x - half, x, x + half] {
        match layout.surface_biome(probe) {
            Some(Surface::Ocean) | Some(Surface::Desert) => return None,
            _ => {}
        }
    }
    // Clear of spawn, so nobody arrives underwater, and clear of the dungeon's mouth.
    if (x - layout.spawn_x).abs() < 60 || (x - layout.dungeon_x).abs() < 80 {
        return None;
    }

    // Level ground. A lake on a slope empties itself down the low side, and nothing settles it
    // afterwards at generation time.
    let ground = *heights.get(x as usize)?;
    let mut lowest = ground;
    let mut highest = ground;
    for probe in (x - half)..=(x + half) {
        let h = *heights.get(probe.max(0) as usize)?;
        lowest = lowest.min(h);
        highest = highest.max(h);
    }
    if highest - lowest > MAX_SLOPE {
        return None;
    }

    let depth = rand.next_range(4, MAX_DEPTH);
    let surface = highest;

    // The floor has to be solid all the way under, or the lake drains into whatever is below it.
    // Checked *before* anything is carved, so a rejected site is left exactly as it was.
    for probe in (x - half)..=(x + half) {
        for dy in 0..=depth + 2 {
            if !world.tile(probe, surface + dy).is_active() {
                return None;
            }
        }
    }

    // --- carve ---------------------------------------------------------------------------------
    //
    // A saucer: deepest in the middle, shallowing to nothing at the rim, so the banks slope in
    // rather than the lake being a rectangular hole with water in it.
    for probe in (x - half)..=(x + half) {
        let across = f64::from((probe - x).abs()) / f64::from(half.max(1));
        let here = ((1.0 - across * across) * f64::from(depth)).round() as i32;
        if here <= 0 {
            continue;
        }
        for dy in 0..here {
            let y = surface + dy;
            let mut tile = Tile::AIR;
            // Water sits in the hole; the rim tile stays dry so the bank is visible.
            tile.liquid = 255;
            tile.liquid_kind = Liquid::Water;
            world.set_tile(probe, y, tile);
        }
        // The bed. Sand under a lake is what vanilla does, and it is what makes the edge read as
        // a shore rather than a hole in the dirt.
        let bed_y = surface + here;
        let bed = world.tile(probe, bed_y);
        if bed.is_active() {
            let mut sand = bed;
            sand.block = tiles::SAND;
            world.set_tile(probe, bed_y, sand);
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::worldgen::layout::Layout;

    /// Flat ground with solid rock beneath it — what a lake needs.
    fn flatland(width: i32, ground: i32) -> (World, Vec<i32>) {
        let mut world = World::empty(width, 400, "lakes");
        for x in 0..width {
            for y in ground..(ground + 120) {
                world.set_tile(x, y, Tile::block(tiles::DIRT));
            }
        }
        (world, vec![ground; width as usize])
    }

    fn layout_for(width: i32) -> Layout {
        let mut rand = UnifiedRandom::new(1);
        Layout::plan(width, 400, &mut rand)
    }

    #[test]
    fn a_lake_holds_water_and_has_sloping_banks() {
        let (mut world, heights) = flatland(1200, 150);
        let layout = layout_for(1200);
        let mut rand = UnifiedRandom::new(9);
        let made = carve(&mut world, &layout, &heights, &mut rand);
        assert!(made > 0, "flat ground should take a lake");

        let mut wet = 0;
        for x in 0..world.width() {
            for y in 140..200 {
                if world.tile(x, y).liquid > 0 {
                    wet += 1;
                }
            }
        }
        assert!(wet > 40, "a lake should hold real water, got {wet} tiles");
    }

    /// The rule that matters most: a lake on a slope empties itself, and nothing settles it here.
    #[test]
    fn no_lake_is_carved_into_a_hillside() {
        let width = 1200;
        let mut world = World::empty(width, 400, "slope");
        // Ground that falls away steeply — a tile of drop every two across, so even the
        // narrowest lake spans more height than its own depth and would empty down the low side.
        // A *gentle* slope is deliberately still allowed: a saucer deeper than the drop holds.
        let heights: Vec<i32> = (0..width).map(|x| 120 + x / 2).collect();
        for x in 0..width {
            for y in heights[x as usize]..(heights[x as usize] + 120) {
                world.set_tile(x, y, Tile::block(tiles::DIRT));
            }
        }
        let layout = layout_for(width);
        let mut rand = UnifiedRandom::new(4);
        assert_eq!(
            carve(&mut world, &layout, &heights, &mut rand),
            0,
            "a hillside must not take a lake; the water would run out of the low side"
        );
    }

    /// A site with a cave under it is rejected *before* anything is carved, so a refusal leaves
    /// the world exactly as it was.
    #[test]
    fn a_hollow_floor_is_refused_without_disturbing_it() {
        let (mut world, heights) = flatland(1200, 150);
        // Hollow out everything just below the surface, so no site has a solid floor.
        for x in 0..1200 {
            for y in 152..170 {
                world.set_tile(x, y, Tile::AIR);
            }
        }
        let before: Vec<_> = (0..1200).map(|x| world.tile(x, 151)).collect();
        let layout = layout_for(1200);
        let mut rand = UnifiedRandom::new(2);

        assert_eq!(carve(&mut world, &layout, &heights, &mut rand), 0);
        for (x, was) in before.iter().enumerate() {
            assert_eq!(&world.tile(x as i32, 151), was, "column {x} was disturbed");
        }
    }
}
