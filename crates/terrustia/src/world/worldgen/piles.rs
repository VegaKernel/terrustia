//! Piles: the large 3×2 kind, and the small 1×1/2×1 kind scattered around them.
//!
//! Two different placers, because vanilla uses two different placers. Large piles (tile 186 for
//! most biomes, 187 for a sandier variant) go through the generic `PlaceTile(x, y, type,
//! mute: true, forced: false, -1, style)` — confirmed by reading the `Piles` pass
//! (`WorldGen.cs:18918`) before source access to the decompiled tree was lost partway through
//! this task — so they route through `TileObjectData`, same as statues, and
//! [`super::place_object::place_object`] is correct for them. Small piles are placed by the
//! dedicated `PlaceSmallPile` (`WorldGen.cs:47221`, read in full before the source was lost),
//! whose frame arithmetic is transcribed directly below, the same way `pots.rs` transcribes
//! `PlacePot`: `frameY = pileSize * 18`, `frameX = pileStyle * 18` (a 1×1 pile) or
//! `pileStyle * 36` with a second tile at `+18` (a 2×1 pile, `pileSize == 1`).
//!
//! **The ground-type → style-range table below is not independently verified against source.**
//! The `Piles` pass body itself (as opposed to the two placer functions above, which were read in
//! full) was not read before the decompiled tree became unavailable mid-session; this table is
//! carried over from an earlier sizing pass's notes rather than confirmed line-by-line the way
//! the rest of this module's transcriptions are. Worth a follow-up pass with source access to
//! confirm the exact tile ids once the tree is available again.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::Tile;

use super::layout::Layout;
use super::place_object::place_object;
use crate::world::World;

/// The two large-pile tiles vanilla has. Which ground type maps to which is inferred (see
/// `large_style_range`'s doc comment) rather than read from source, so the names stay neutral —
/// "SAND" would assert a meaning that has not actually been confirmed.
const LARGE_PILE_A: u16 = 186;
const LARGE_PILE_B: u16 = 187;
const SMALL_PILE: u16 = 185;
/// `TileID.Boulder` — `InvalidTileForPilesOrSpeleothems` refuses a floor of this type.
const BOULDER: u16 = 26;

/// `PlaceSmallPile`, transcribed. `size` is 0 for a single 1×1 tile, 1 for a 2×1 pair.
fn place_small_pile(world: &mut World, x: i32, y: i32, style: i32, size: i32) -> bool {
    if world.tile(x, y).liquid > 0 {
        return false;
    }
    let floor_ok = |world: &World, fx: i32| {
        let floor = world.tile(fx, y + 1);
        floor.is_active()
            && terrustia_proto::tile_solid::solid(floor.block)
            && floor.block != BOULDER
    };
    if size == 1 {
        if !floor_ok(world, x)
            || !floor_ok(world, x + 1)
            || world.tile(x, y).is_active()
            || world.tile(x + 1, y).is_active()
        {
            return false;
        }
        let frame_y = 18i16;
        let frame_x = (style * 36) as i16;
        world.set_tile(x, y, Tile::framed(SMALL_PILE, frame_x, frame_y));
        world.set_tile(x + 1, y, Tile::framed(SMALL_PILE, frame_x + 18, frame_y));
        true
    } else {
        if !floor_ok(world, x) || world.tile(x, y).is_active() {
            return false;
        }
        world.set_tile(x, y, Tile::framed(SMALL_PILE, (style * 18) as i16, 0));
        true
    }
}

/// Ground type → (which large-pile tile, style range), by what `pileStyle` a floor rolls.
///
/// **Not independently verified against source** — see the module doc. The (tile, range) split
/// below is an inference, not a transcription: the sizing notes this table came from state a
/// style range per ground type but only say which of the two pile tiles (186/187) is used for
/// one of them ("dirt/stone/moss ... on 187"). The rest is inferred from the ranges being
/// contiguous in blocks — dirt/stone/moss (23-29) and sandstone (29-35) butt up against each
/// other with no gap, which is the kind of thing that happens when two ground types share a
/// tile's style space, so both are placed on 187 here; everything else goes on 186. The specific
/// tile *ids* below (which numeric id means "moss" versus "granite" and so on) are likewise
/// carried over rather than confirmed against `TileID.cs`. Flagged for a follow-up pass once
/// source access is available again — this is the one place in this file where that matters.
fn large_style_range(ground: u16) -> Option<(u16, i32, i32)> {
    Some(match ground {
        2 | 1 | 59 => (LARGE_PILE_B, 23, 29), // dirt, stone, moss
        53 => (LARGE_PILE_B, 29, 35),         // sandstone
        147 | 161 => (LARGE_PILE_A, 26, 32),  // snow, ice
        60 => (LARGE_PILE_A, 0, 6),           // jungle
        58 | 57 => (LARGE_PILE_A, 6, 9),      // ash, hellstone
        226 => (LARGE_PILE_A, 18, 23),        // lihzahrd
        70 => (LARGE_PILE_A, 32, 35),         // mushroom
        123 => (LARGE_PILE_A, 35, 41),        // granite
        367 => (LARGE_PILE_A, 41, 47),        // marble
        41 | 43 | 44 => (LARGE_PILE_A, 0, 7), // dungeon brick
        _ => return None,
    })
}

/// Scatter large piles, each with a chance of scattering a few small ones around it.
pub fn scatter(world: &mut World, layout: &Layout, rng: &mut SmallRng) -> (usize, usize) {
    let attempts = ((layout.width as i64 * world.height() as i64) as f64 * 0.0004) as usize;
    let (mut large, mut small) = (0, 0);
    let bottom = (world.height() - 20).max(21);

    for _ in 0..attempts {
        let x = rng.random_range(20..(layout.width - 20).max(21));
        let mut y = rng.random_range(layout.surface.max(1)..bottom);
        while y < bottom && !world.tile(x, y).is_active() {
            y += 1;
        }
        if y >= bottom {
            continue;
        }
        let Some((block, lo, hi)) = large_style_range(world.tile(x, y).block) else {
            continue;
        };
        let style = rng.random_range(lo..hi);
        // The floor tile itself is the anchor's `below`; the pile sits just above it.
        if place_object(world, x, y - 1, block, style, -1) {
            large += 1;
            for _ in 0..rng.random_range(1..5) {
                let sx = x + rng.random_range(-10..=10);
                let sy = y - 1 + rng.random_range(-2..=2);
                let size = i32::from(rng.random_range(0..2) == 0);
                let pstyle = rng.random_range(12..36);
                if place_small_pile(world, sx, sy, pstyle, size) {
                    small += 1;
                }
            }
        }
    }
    (large, small)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn a_small_pile_matches_placesmallpiles_own_frame_arithmetic() {
        let mut world = World::empty(40, 40, "piles");
        for x in 0..40 {
            world.set_tile(x, 20, Tile::block(1));
        }
        assert!(place_small_pile(&mut world, 10, 19, 5, 0));
        let tile = world.tile(10, 19);
        assert_eq!(tile.frame_x, 5 * 18);
        assert_eq!(tile.frame_y, 0);

        assert!(place_small_pile(&mut world, 20, 19, 3, 1));
        let left = world.tile(20, 19);
        let right = world.tile(21, 19);
        assert_eq!(left.frame_x, 3 * 36);
        assert_eq!(left.frame_y, 18);
        assert_eq!(right.frame_x, 3 * 36 + 18);
    }

    #[test]
    fn a_small_pile_refuses_a_boulder_floor() {
        let mut world = World::empty(40, 40, "piles");
        world.set_tile(10, 20, Tile::framed(BOULDER, 0, 0));
        assert!(!place_small_pile(&mut world, 10, 19, 0, 0));
    }

    #[test]
    fn piles_scatter_across_a_generated_world() {
        let mut world = World::empty(4200, 400, "piles");
        for x in 0..4200 {
            for y in 300..400 {
                world.set_tile(x, y, Tile::block(1));
            }
        }
        let mut rand = super::super::rand::UnifiedRandom::new(5);
        let layout = super::super::layout::Layout::plan(4200, 400, &mut rand);
        let mut rng = SmallRng::seed_from_u64(6);
        let (large, small) = scatter(&mut world, &layout, &mut rng);
        assert!(large > 0, "a wide flat floor should take large piles");
        let _ = small;
    }
}
