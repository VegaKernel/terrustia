//! Piles: the large 3×2 kind, and the small 1×1/2×1 kind scattered around them.
//!
//! Two different placers, because vanilla uses two different placers. Large piles (tile 186 for
//! most biomes, 187 for a sandier variant) go through the generic `PlaceTile(x, y, type,
//! mute: true, forced: false, -1, style)`, so they route through `TileObjectData`, same as
//! statues, and [`super::place_object::place_object`] is correct for them. Small piles are placed
//! by the dedicated `PlaceSmallPile` (`WorldGen.cs:47229-47287`), whose frame arithmetic is
//! transcribed directly below, the same way `pots.rs` transcribes `PlacePot`: `frameY = pileSize *
//! 18`, `frameX = pileStyle * 18` (a 1×1 pile) or `pileStyle * 36` with a second tile at `+18` (a
//! 2×1 pile, `pileSize == 1`).
//!
//! The ground-type → style-range table below (`large_pile_roll`) is transcribed from the `Piles`
//! pass's primary loop, `WorldGen.cs:18942-19066` — read in full once decompiled source access was
//! restored, replacing the version of this file that carried the table over from sizing notes
//! rather than source. Two things it does not reproduce, both noted at `large_pile_roll`: vanilla's
//! dungeon-wall gate (terrustia's generator does not track which walls are "dungeon" walls) and its
//! 1-in-3 placement abandonment near one. Vanilla also runs several *more* pile-scattering loops in
//! the same pass — a deep-cavern band, a wall-backed surface band, and a large standalone
//! small-pile-only band — each rolling broadly similar but not identical style tables for the same
//! ground types. This file transcribes the first (and by attempt count, the primary) loop only;
//! reproducing all of vanilla's loops as separate depth/context-gated passes would be a
//! structural rewrite, not a table fix, so it is not attempted here.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::{Tile, TileFlags};

use super::layout::Layout;
use super::place_object::place_object;
use super::tiles;
use crate::world::World;

const LARGE_PILE_A: u16 = 186; // TileID.LargePiles
const LARGE_PILE_B: u16 = 187; // TileID.LargePiles2
const SMALL_PILE: u16 = 185; // TileID.SmallPiles
/// Not in `tiles.rs`: `WoodBlock`. Only used here, as one of the "plain" floor types.
const WOOD_BLOCK: u16 = 30;
/// Not in `tiles.rs`: `Platforms`.
const PLATFORMS: u16 = 19;
/// Not in `tiles.rs`: `BreakableIce`. Grouped with snow/ice below.
const BREAKABLE_ICE: u16 = 162;
/// Not in `tiles.rs`: `DesertFossil`. Grouped with the sandstone family below.
const DESERT_FOSSIL: u16 = 404;

/// `Main.tileMoss` (`Main.cs:7146-7186`). Terrustia does not generate any of these yet — moss is
/// unstarted Tier 3 work — so this can never actually match today, but it costs nothing to have
/// right for when it does.
fn is_moss(ground: u16) -> bool {
    matches!(
        ground,
        179 | 180 | 181 | 182 | 183 | 381 | 534 | 536 | 539 | 625 | 627
    )
}

/// `TileID.Sets.Boulders` (`TileID.cs:195`) — every boulder variant `PlaceSmallPile`'s 2×1 case
/// refuses as a floor, via `InvalidTileForPilesOrSpeleothems` (`WorldGen.cs:39380`). The 1×1 case
/// does not call this check at all (see `place_small_pile`).
fn is_boulder(block: u16) -> bool {
    matches!(
        block,
        138 | 484 | 664 | 665 | 711 | 712 | 713 | 714 | 715 | 716
    )
}

/// Roll a large pile's (tile, style) for a floor tile, transcribed from the `Piles` pass's
/// primary loop, `WorldGen.cs:18963-19030`. Every branch is a direct `if`-override in source,
/// applied in the same order — a later branch can overwrite an earlier one for the same floor
/// tile (e.g. snow always wins over the plain default, since it's checked after it).
///
/// Not reproduced: the `Main.wallDungeon` gate and its 1-in-3 placement abandonment, since
/// terrustia's generator does not track which walls are dungeon walls. Everything else — every
/// ground-type branch, every exact `genRand` call, and the final 1-in-75 style-17 crossover —
/// matches source.
fn large_pile_roll(ground: u16, y: i32, world_height: i32, rng: &mut SmallRng) -> (u16, i32) {
    let mut tile = LARGE_PILE_A;
    let mut style = rng.random_range(0..22);
    if (16..22).contains(&style) {
        style = rng.random_range(0..22);
    }
    if (ground == tiles::DIRT || ground == tiles::STONE || is_moss(ground))
        && rng.random_range(0..5) == 0
    {
        style = rng.random_range(23..29);
        tile = LARGE_PILE_B;
    }
    let very_deep = y > world_height - 300;
    if very_deep
        || matches!(
            ground,
            WOOD_BLOCK | PLATFORMS | tiles::EBONSTONE | tiles::CRIMSTONE
        )
    {
        style = rng.random_range(0..7);
        tile = LARGE_PILE_A;
    }
    if matches!(ground, tiles::SNOW | tiles::ICE | BREAKABLE_ICE) {
        style = rng.random_range(26..32);
        tile = LARGE_PILE_A;
    }
    if ground == tiles::JUNGLE_GRASS {
        tile = LARGE_PILE_B;
        style = rng.random_range(0..6);
    }
    if matches!(ground, tiles::ASH | tiles::HELLSTONE) && rng.random_range(0..3) < 2 {
        tile = LARGE_PILE_B;
        style = rng.random_range(6..9);
    }
    if ground == tiles::LIHZAHRD_BRICK {
        tile = LARGE_PILE_B;
        style = rng.random_range(18..23);
    }
    if ground == tiles::MUSHROOM_GRASS {
        style = rng.random_range(32..35);
        tile = LARGE_PILE_A;
    }
    if matches!(
        ground,
        tiles::SANDSTONE | tiles::HARDENED_SAND | DESERT_FOSSIL
    ) {
        style = rng.random_range(29..35);
        tile = LARGE_PILE_B;
    }
    if ground == tiles::GRANITE {
        style = rng.random_range(35..41);
        tile = LARGE_PILE_B;
    }
    if ground == tiles::MARBLE {
        style = rng.random_range(41..47);
        tile = LARGE_PILE_B;
    }
    if tile == LARGE_PILE_A && (7..=15).contains(&style) && rng.random_range(0..75) == 0 {
        tile = LARGE_PILE_B;
        style = 17;
    }
    (tile, style)
}

/// `PlaceSmallPile`, transcribed (`WorldGen.cs:47229-47287`). `size` is 0 for a single 1×1 tile, 1
/// for a 2×1 pair. The two sizes use different floor checks in source: the 2×1 case requires
/// `SolidTile2` under both tiles and refuses a boulder floor; the 1×1 case only requires
/// `SolidTile2` under itself, with no boulder check at all.
/// `pub(crate)` rather than private: `spider_caves.rs` reuses this directly for the small piles
/// `Spread.Spider` (`WorldGen.cs:3697`) scatters inside a spider cave.
pub(crate) fn place_small_pile(world: &mut World, x: i32, y: i32, style: i32, size: i32) -> bool {
    if world.tile(x, y).liquid > 0 {
        return false;
    }
    // `SolidTile2` (`WorldGen.cs:70673`): active, solid, unsloped, not a half brick.
    let solid_tile_2 = |world: &World, fx: i32, fy: i32| {
        let t = world.tile(fx, fy);
        t.is_active()
            && terrustia_proto::tile_solid::solid(t.block)
            && t.slope == 0
            && !t.flags.has(TileFlags::HALF_BRICK)
    };
    if size == 1 {
        if !solid_tile_2(world, x, y + 1)
            || !solid_tile_2(world, x + 1, y + 1)
            || world.tile(x, y).is_active()
            || world.tile(x + 1, y).is_active()
            || is_boulder(world.tile(x, y + 1).block)
            || is_boulder(world.tile(x + 1, y + 1).block)
        {
            return false;
        }
        let frame_y = 18i16;
        let frame_x = (style * 36) as i16;
        world.set_tile(x, y, Tile::framed(SMALL_PILE, frame_x, frame_y));
        world.set_tile(x + 1, y, Tile::framed(SMALL_PILE, frame_x + 18, frame_y));
        true
    } else {
        if !solid_tile_2(world, x, y + 1) || world.tile(x, y).is_active() {
            return false;
        }
        world.set_tile(x, y, Tile::framed(SMALL_PILE, (style * 18) as i16, 0));
        true
    }
}

/// Scatter large piles, each with a chance of scattering a few small ones around it.
pub fn scatter(world: &mut World, layout: &Layout, rng: &mut SmallRng) -> (usize, usize) {
    let attempts = ((layout.width as i64 * world.height() as i64) as f64 * 0.0004) as usize;
    let (mut large, mut small) = (0, 0);
    let bottom = (world.height() - 20).max(21);
    let world_height = world.height();

    for _ in 0..attempts {
        let x = rng.random_range(20..(layout.width - 20).max(21));
        let mut y = rng.random_range(layout.surface.max(1)..bottom);
        while y < bottom && !world.tile(x, y).is_active() {
            y += 1;
        }
        if y >= bottom {
            continue;
        }
        let (block, style) = large_pile_roll(world.tile(x, y).block, y, world_height, rng);
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
    fn a_2x1_small_pile_refuses_a_boulder_floor() {
        let mut world = World::empty(40, 40, "piles");
        world.set_tile(10, 20, Tile::framed(138, 0, 0)); // TileID.Boulder
        world.set_tile(11, 20, Tile::block(1));
        assert!(!place_small_pile(&mut world, 10, 19, 0, 1));
    }

    #[test]
    fn a_1x1_small_pile_does_not_check_for_a_boulder_floor() {
        // `PlaceSmallPile`'s `pileSize == 0` branch never calls `InvalidTileForPilesOrSpeleothems`
        // — only the 2×1 branch does. A boulder floor is otherwise `SolidTile2`-valid (active,
        // solid, unsloped, no half brick), so the 1×1 case must accept it.
        let mut world = World::empty(40, 40, "piles");
        world.set_tile(10, 20, Tile::framed(138, 0, 0)); // TileID.Boulder
        assert!(place_small_pile(&mut world, 10, 19, 0, 0));
    }

    #[test]
    fn a_small_pile_refuses_a_sloped_floor() {
        // `SolidTile2` requires `slope() == 0`; the older floor check here did not, so a sloped
        // floor would previously have accepted a pile it should have refused.
        let mut world = World::empty(40, 40, "piles");
        let mut sloped = Tile::block(1);
        sloped.slope = 1;
        world.set_tile(10, 20, sloped);
        assert!(!place_small_pile(&mut world, 10, 19, 0, 0));
    }

    #[test]
    fn large_pile_style_matches_vanillas_biome_ranges() {
        // Vanilla's exact ranges, `WorldGen.cs:18986-19024`: snow/ice always lands in [26, 32) on
        // tile 186, and sandstone always lands in [29, 35) on tile 187 — regardless of which of
        // the two "plain floor" rolls happened first, since these branches are checked afterward
        // and unconditionally override them.
        let mut rng = SmallRng::seed_from_u64(1);
        for _ in 0..200 {
            let (tile, style) = large_pile_roll(tiles::SNOW, 100, 1200, &mut rng);
            assert_eq!(tile, LARGE_PILE_A);
            assert!((26..32).contains(&style));

            let (tile, style) = large_pile_roll(tiles::SANDSTONE, 100, 1200, &mut rng);
            assert_eq!(tile, LARGE_PILE_B);
            assert!((29..35).contains(&style));

            let (tile, style) = large_pile_roll(tiles::MARBLE, 100, 1200, &mut rng);
            assert_eq!(tile, LARGE_PILE_B);
            assert!((41..47).contains(&style));
        }
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
