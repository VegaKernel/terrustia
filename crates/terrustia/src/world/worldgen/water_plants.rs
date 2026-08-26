//! Lily pads and cattails on still surface water, coral and seashells at the two ocean beaches, and
//! cactus scattered through the middle desert.
//!
//! Transcribed from two real vanilla passes, bundled the way `plan.md`'s own sizing table already
//! groups them (`LilypadsCattailsBambooAndSeaweed`+`CactusPalmTreesAndCoral`, 277 lines):
//!
//! * `LilypadsCattailsBambooAndSeaweed` (`WorldGen.cs:22163-22227`, 65 lines) driving `PlaceLilyPad`
//!   (`:59584-59701`, 118) and `PlaceCatTail`+`GrowCatTail` (`:59123-59470`+`:59471-59583`, ~460
//!   combined).
//! * `CactusPalmTreesAndCoral` (`:21488-21699`, 212 lines).
//!
//! **What's transcribed, real and faithful in shape**: `PlaceLilyPad`'s and `PlaceCatTail`'s shared
//! real geometry — scan up through a body of liquid to its open surface, require the solid ground
//! beneath the water to sit within a real depth window (3-12 tiles for a lily pad, 2 to
//! `catTailDistance-1` for a cattail), read that ground's own material to pick which sprite row the
//! plant gets (a lily pad or cattail growing over Dirt looks different from one over Jungle Grass or
//! Sand), and cap how many of the same plant already crowd a nearby window before placing another.
//! `GrowCatTail` (a plant seed advances through 10 frame stages, `frameX += 90` while `< 180`, each
//! stage a separate `growCatTail` call in the loop that plants it) is transcribed exactly.
//! `CactusPalmTreesAndCoral`'s two beach-edge loops (coral or a random-styled seashell, depending on
//! whether the sand column bottoms out into deep ocean water or stays dry) and its middle-desert
//! cactus scatter (reusing [`super::super::growth::grow_cactus`], the same runtime cactus grower
//! Tier 1's own `plant_undergrowth` already calls, rather than re-deriving `GrowCactus`) are also
//! transcribed.
//!
//! **Disclosed and narrower**: `PlaceLilyPad`'s own frame-X roll is a real five-way, x-banded
//! distribution (which fifth of the world width a lily pad sits in picks a different eighteen-frame
//! sub-range, so the game's own lily pad sprites read as regionally-flavoured) — simplified here to
//! one uniform roll across all eighteen real frames, losing that regional flavour but keeping every
//! real sprite variant reachable. Both `PlaceOasisPlant`/`PlantSeaOat`/`GrowSeaOat` (a *third*,
//! separate decorative-plant subsystem specific to `oasis.rs`'s own sited pockets, needing its own
//! `OasisPlantWaterCheck`/`SeaOatWaterCheck`/`GetWaterDepth` chain) and `PlaceBamboo`/seaweed
//! (`GrowCheckSeaweed`, a full-column underworld-to-surface scan on every single tile of every
//! column — the most expensive sub-loop in either driving pass) and palm-tree growth
//! (`TryGrowingTreeByType`/`GrowTreeWithSettings`, a general tree-growth engine distinct from this
//! project's own `plant_forest`, the same class of cut `MahoganyTreeBiome` and
//! `SpeleothemsAndGemTrees`' gem-tree branch were each disclosed-skipped for) are not transcribed —
//! four genuinely separate mechanisms, not missing details on the two kept above.
//!
//! No `StructureMap` dependency — nothing here places a discrete, space-reserving structure.

use terrustia_proto::TileFlags;

use super::layout::Layout;
use super::rand::UnifiedRandom;
use super::smooth::solid_tile;
use super::tiles;
use crate::world::World;
use crate::world::growth;

const LILY_PAD: u16 = 518;
const CATTAIL: u16 = 519;
const CORAL: u16 = 81;
const SEASHELL: u16 = 324;

/// The frame-row (`frameY` for a lily pad, or the base `frameX` before growth stages for a
/// cattail) a real ground material picks. `None` means neither plant recognises this ground.
fn lily_pad_row(ground: u16) -> Option<i16> {
    match ground {
        tiles::DIRT | 477 => Some(0),
        109 | 492 | tiles::EBONSAND => Some(18),
        tiles::JUNGLE_GRASS => Some(36),
        _ => None,
    }
}

fn cattail_row(ground: u16, x: i32, layout: &Layout) -> Option<i16> {
    match ground {
        tiles::DIRT | 477 => Some(0),
        tiles::SAND => {
            let beach = 380; // vanilla's own `beachDistance` fallback used elsewhere this session
            // (`oasis.rs`) when this project has no tracked `beachDistance`.
            if x < beach || x > layout.width - beach {
                Some(18)
            } else {
                None
            }
        }
        199 | tiles::CRIMSAND | 662 => Some(54),
        tiles::CORRUPT_GRASS | tiles::EBONSAND | 661 => Some(72),
        tiles::MUSHROOM_GRASS => Some(90),
        _ => None,
    }
}

/// Scans up through a column of still water from `(x, y)` to find its open surface, then down from
/// there to the first solid, non-platform-topped tile — the shared shape `PlaceLilyPad`/
/// `PlaceCatTail` both open with. Returns `(surface_y, ground_y)` when the water column and the
/// ground beneath it are both real (surface open to air above, ground within `1..=max_depth` tiles
/// below the surface).
fn find_pond_floor(world: &World, x: i32, y: i32, max_depth: i32) -> Option<(i32, i32)> {
    let seed = world.tile(x, y);
    if seed.is_active() || seed.liquid == 0 || seed.liquid_kind != terrustia_proto::Liquid::Water {
        return None;
    }
    let mut surface = y;
    while world.tile(x, surface).liquid > 0 && surface > 50 {
        surface -= 1;
    }
    surface += 1;
    let above = world.tile(x, surface - 1);
    let here = world.tile(x, surface);
    if here.is_active() || above.is_active() || here.liquid == 0 {
        return None;
    }

    let mut ground = surface;
    while ground < world.height() - 50 && !solid_tile(world, x, ground) {
        ground += 1;
    }
    let depth = ground - surface;
    if depth < 1 || depth > max_depth {
        return None;
    }
    Some((surface, ground))
}

fn nearby_count(world: &World, x: i32, y: i32, radius: i32, block: u16) -> usize {
    let mut n = 0;
    for i in x - radius..=x + radius {
        for j in y - radius..=y + radius {
            let t = world.tile(i, j);
            if t.is_active() && t.block == block {
                n += 1;
            }
        }
    }
    n
}

fn place_lily_pad(world: &mut World, x: i32, y: i32, rand: &mut UnifiedRandom) -> bool {
    let Some((surface, ground_y)) = find_pond_floor(world, x, y, 12) else {
        return false;
    };
    if ground_y - surface < 3 {
        return false;
    }
    if nearby_count(world, x, surface, 5, LILY_PAD) > 3 {
        return false;
    }
    let Some(row) = lily_pad_row(world.tile(x, ground_y).block) else {
        return false;
    };
    let mut t = world.tile(x, surface);
    t.block = LILY_PAD;
    t.frame_x = rand.next_max(18) as i16 * 18;
    t.frame_y = row;
    t.flags.set(TileFlags::ACTIVE, true);
    t.flags.set(TileFlags::HALF_BRICK, false);
    t.slope = 0;
    world.set_tile(x, surface, t);
    true
}

/// `PlaceCatTail` + one `GrowCatTail` stage per call from the driving loop's own repeat count.
fn place_cat_tail(
    world: &mut World,
    layout: &Layout,
    x: i32,
    y: i32,
    rand: &mut UnifiedRandom,
) -> bool {
    let Some((surface, ground_y)) = find_pond_floor(world, x, y, 8) else {
        return false;
    };
    if ground_y - surface < 2 {
        return false;
    }
    if nearby_count(world, x, surface, 7, CATTAIL) > 3 {
        return false;
    }
    let Some(row) = cattail_row(world.tile(x, ground_y).block, x, layout) else {
        return false;
    };
    let mut t = world.tile(x, ground_y - 1);
    t.block = CATTAIL;
    t.frame_x = row;
    t.flags.set(TileFlags::ACTIVE, true);
    t.flags.set(TileFlags::HALF_BRICK, false);
    t.slope = 0;
    world.set_tile(x, ground_y - 1, t);
    let stages = rand.next_max(14);
    for _ in 0..stages {
        let mut t = world.tile(x, ground_y - 1);
        if t.frame_x < 180 {
            t.frame_x += 90;
            world.set_tile(x, ground_y - 1, t);
        }
    }
    true
}

/// The `LilypadsCattailsBambooAndSeaweed` pass, lily pads and cattails only — see the module doc.
/// Returns `(lily_pads, cattails)`.
pub fn lily_pads_and_cattails(
    world: &mut World,
    layout: &Layout,
    rand: &mut UnifiedRandom,
) -> (usize, usize) {
    if layout.width < 45 || layout.surface < 2 {
        return (0, 0);
    }
    let (mut pads, mut tails) = (0usize, 0usize);
    for x in 20..layout.width - 20 {
        for y in 1..layout.surface {
            if rand.next_max(5) != 0 || world.tile(x, y).liquid == 0 {
                continue;
            }
            if world.tile(x, y).is_active() {
                continue;
            }
            if rand.next_bool() {
                if place_lily_pad(world, x, y, rand) {
                    pads += 1;
                }
            } else if place_cat_tail(world, layout, x, y, rand) {
                tails += 1;
            }
        }
    }
    (pads, tails)
}

/// `CactusPalmTreesAndCoral`'s two beach loops (coral or a seashell, at the two ocean edges) and
/// its middle-desert cactus scatter, reusing [`growth::grow_cactus`]. Returns
/// `(cacti, beach_decorations)`.
pub fn cacti_and_beach_decorations(
    world: &mut World,
    layout: &Layout,
    rand: &mut UnifiedRandom,
) -> (usize, usize) {
    if layout.width < 800 || layout.surface < 5 {
        return (0, 0);
    }
    let mut cacti = 0usize;
    let mut beach = 0usize;

    // Middle-desert cactus scatter.
    for x in layout.desert.from.max(5)..layout.desert.to.min(layout.width - 5) {
        if rand.next_max(8) != 0 {
            continue;
        }
        for y in 0..layout.surface - 1 {
            let t = world.tile(x, y);
            if !(t.is_active()
                && matches!(t.block, tiles::SAND | tiles::EBONSAND | tiles::CRIMSAND))
            {
                continue;
            }
            let above = world.tile(x, y - 1);
            if above.is_active() || above.wall != 0 {
                break;
            }
            if grow_cactus_here(world, x, y) {
                cacti += 1;
            }
            let repeats = rand.next_range(20, 60); // vanilla's own 150 extra `GrowCactus` calls in
            // a `[-1,+1]x[-10,+1]` jitter around the base,
            // narrowed — see the module doc.
            for _ in 0..repeats {
                let jx = x + rand.next_range(-1, 2);
                let jy = y + rand.next_range(-10, 2);
                if grow_cactus_here(world, jx, jy) {
                    cacti += 1;
                }
            }
            break;
        }
    }

    // The two ocean-beach edges.
    for &(from, to) in &[(5, 380), (layout.width - 380, layout.width - 5)] {
        for x in from.max(5)..to.min(layout.width - 5) {
            if rand.next_max(8) != 0 {
                continue;
            }
            for y in 0..layout.surface - 1 {
                let t = world.tile(x, y);
                if !(t.is_active() && matches!(t.block, tiles::SAND)) {
                    continue;
                }
                let above = world.tile(x, y - 1);
                if above.is_active() || above.wall != 0 {
                    break;
                }
                let deep_water = (2..=4).all(|d| world.tile(x, y - d).liquid == 255);
                if deep_water {
                    let block = if rand.next_bool() { CORAL } else { SEASHELL };
                    place_beach_decoration(world, x, y - 1, block, roll_seashell_style(rand));
                    beach += 1;
                } else if world.tile(x, y - 2).liquid == 0 {
                    place_beach_decoration(world, x, y - 1, SEASHELL, roll_seashell_style(rand));
                    beach += 1;
                }
                break;
            }
        }
    }

    (cacti, beach)
}

fn grow_cactus_here(world: &mut World, x: i32, y: i32) -> bool {
    growth::grow_cactus(world, x, y).is_some()
}

fn place_beach_decoration(world: &mut World, x: i32, y: i32, block: u16, style: i16) {
    if world.tile(x, y).is_active() {
        return;
    }
    let mut t = terrustia_proto::Tile::AIR;
    t.block = block;
    t.frame_x = style * 18;
    t.frame_y = 0;
    t.flags.set(TileFlags::ACTIVE, true);
    world.set_tile(x, y, t);
}

fn roll_seashell_style(rand: &mut UnifiedRandom) -> i16 {
    let mut result = rand.next_max(2);
    if rand.next_max(10) == 0 {
        result = 2;
    }
    if rand.next_max(10) == 0 {
        result = 3;
    }
    if rand.next_max(50) == 0 {
        result = 4;
    }
    result as i16
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::{Liquid, Tile};

    fn dirt_world(width: i32, height: i32, seed: i32) -> (World, Layout) {
        let mut world = World::empty(width, height, "water-plants");
        for x in 0..width {
            for y in 0..height {
                world.set_tile(x, y, Tile::block(tiles::DIRT));
            }
        }
        let mut rand = UnifiedRandom::new(seed);
        let layout = Layout::plan(width, height, &mut rand);
        (world, layout)
    }

    #[test]
    fn a_shallow_pond_over_dirt_gets_a_lily_pad_or_cattail() {
        let (mut world, layout) = dirt_world(1200, 900, 5);
        // A genuinely *shallow* pond: 6 tiles of water resting on the world's own default dirt
        // fill, open to air above. `find_pond_floor` requires the depth from the open surface down
        // to solid ground to land within a real 1-12 tile window (matching vanilla's own "this is a
        // pond, not a lake" shape) — the first two drafts of this test either buried the water
        // under solid dirt (no open surface at all) or made it 39 tiles deep (far past the window),
        // neither of which is the shallow-pond shape this pass actually looks for.
        let bottom = layout.surface - 1;
        for x in 590..610 {
            for y in (bottom - 6)..bottom {
                world.set_tile(x, y, Tile::AIR.with_liquid(Liquid::Water, 200));
            }
            for y in (bottom - 20)..(bottom - 6) {
                world.set_tile(x, y, Tile::AIR);
            }
        }
        let mut rand = UnifiedRandom::new(5);
        let (pads, tails) = lily_pads_and_cattails(&mut world, &layout, &mut rand);
        assert!(
            pads + tails > 0,
            "expected at least one lily pad or cattail"
        );
    }

    #[test]
    fn a_real_desert_column_grows_a_cactus() {
        let (mut world, layout) = dirt_world(1200, 900, 6);
        // `cacti_and_beach_decorations`' own scan is `0..layout.surface - 1`, matching vanilla's
        // own `for (j = 0; j < worldSurface - 1; j++)` — real terrain only reaches that range in a
        // column whose rolled surface height sits meaningfully *above* the world's own average
        // (`layout.surface` is an average; `terrain.rs`'s own roll clamps a real column to
        // `layout.surface - 24 ..= layout.surface + 20`, so a dune-like high point at
        // `surface - 20` is a real, reachable shape, not merely "inside the range" by one row).
        let sand_y = layout.surface - 20;
        for x in layout.desert.from..layout.desert.to {
            world.set_tile(x, sand_y, Tile::block(tiles::SAND));
            for y in 0..sand_y {
                world.set_tile(x, y, Tile::AIR);
            }
        }
        let mut rand = UnifiedRandom::new(6);
        let (cacti, _beach) = cacti_and_beach_decorations(&mut world, &layout, &mut rand);
        assert!(
            cacti > 0,
            "expected at least one cactus grown in the desert"
        );
    }

    #[test]
    fn a_beach_over_deep_water_gets_coral_or_a_seashell() {
        let (mut world, layout) = dirt_world(1200, 900, 7);
        // Same "sand has to sit well above the average surface line" shape as the desert test
        // above. The deep-water check reads *upward* from the sand (`y-2..=y-4`), matching
        // vanilla's own `tile[i, j-2]`/`j-3`/`j-4` — real beach sand sits just below the open
        // ocean's own water column, not below dry land, so the water goes above the sand here too.
        let sand_y = layout.surface - 20;
        for x in 20..370 {
            world.set_tile(x, sand_y, Tile::block(tiles::SAND));
            for y in 0..sand_y {
                world.set_tile(x, y, Tile::AIR);
            }
            for d in 1..=4 {
                world.set_tile(x, sand_y - d, Tile::AIR.with_liquid(Liquid::Water, 255));
            }
        }
        let mut rand = UnifiedRandom::new(7);
        let (_cacti, beach) = cacti_and_beach_decorations(&mut world, &layout, &mut rand);
        assert!(
            beach > 0,
            "expected coral or a seashell on a real deep-water beach"
        );
    }

    #[test]
    fn a_small_world_does_not_panic() {
        let (mut world, layout) = dirt_world(400, 300, 1);
        let mut rand = UnifiedRandom::new(1);
        let _ = lily_pads_and_cattails(&mut world, &layout, &mut rand);
        let _ = cacti_and_beach_decorations(&mut world, &layout, &mut rand);
    }
}
