//! Gem caves: pockets of stone recolored into gem ore and lined with a matching gem wall.
//!
//! Transcribed from the `GemCaves` generation pass (`WorldGen.cs:17543-17587`) and the two
//! functions it drives: `gemCave` (`WorldGen.cs:9673`, which rolls which 1-6 gem types this
//! particular pocket may contain) and `Spread.Gem` (`WorldGen.cs:3534`, the wave flood-fill that
//! actually paints the pocket). Site-searching reuses [`super::cave_flood::count`], the same
//! `countTiles`/`nextCount` mechanism `SpiderCaves` also drives off of.
//!
//! `Spread.Gem` is a *second*, different flood fill from `cave_flood`'s — it walks outward in
//! waves (a queue of "this wave's tiles", refilled from what the wave touched) rather than a
//! depth-first stack, because unlike `count` it does real per-tile work (a wall write, a possible
//! tile recolor) and a stack-based order would still visit every reachable tile, just in a
//! different sequence — vanilla's own wave order has no gameplay consequence here, but the
//! wave-queue shape is transcribed anyway rather than swapped for the stack `cave_flood` uses, to
//! keep this a faithful port rather than a reinterpretation of a pass with real random-number
//! consumption per tile.
//!
//! **Site-acceptance deviates from vanilla's own range check, for two related reasons.** Vanilla
//! rejects a candidate whose pocket both undershoots 50 tiles *and* overshoots 300
//! (`cave_flood`'s search cap), has any lava or ice, or never touches a stone tile at all
//! (`rockCount == 0`) — the upper bound exists to avoid siting in vast open spaces (an ocean, the
//! underworld), and `rockCount` exists to confirm the pocket actually borders real rock rather
//! than floating entirely inside some other biome's material.
//!
//! `structures::caves()` produces one large interconnected tunnel network rather than vanilla's
//! mix of small isolated pockets. That breaks both checks the same way: *every* candidate
//! saturates the 300-tile search (rejecting on that basis would reject everywhere, which is
//! exactly the bug this module shipped with before this fix), and the search's stack-based fill
//! spends its whole 300-tile budget wandering the open interior of a much larger network,
//! essentially never reaching an actual stone boundary tile within that budget — measured at 98%
//! of real candidates failing `rockCount == 0` even after the size cap was already fixed. Neither
//! check is measuring what it's meant to measure once the topology it assumes no longer holds, so
//! neither gates acceptance here. What still does: the 50-tile lower bound, lava/ice, and — in
//! `rockCount`'s place — the `y` sample range itself (`layout.rock + 30 ..`), which is the actual
//! guarantee a candidate sits in the rock layer that `rockCount` was a proxy for.
//! [`spread_gem`]'s own doc comment covers the other half of this fix, capping the *decoration*
//! instead of the *site*.

use std::collections::HashSet;

use terrustia_proto::{TileFlags, tile_solid};

use super::cave_flood;
use super::layout::Layout;
use super::rand::UnifiedRandom;
use super::tiles::{self, walls};

/// `TileID.ExposedGems` — the same tile `speleothems.rs`'s own exposed-gem passes place.
const GEM_TILE: u16 = 178;

/// `Gemmable` (`WorldGen.cs:3731`): which active tile types `Spread.Gem` will recolor.
fn gemmable(block: u16) -> bool {
    matches!(
        block,
        0 | tiles::STONE | 40 | tiles::MUD | tiles::JUNGLE_GRASS | tiles::MUSHROOM_GRASS
    ) || matches!(block, tiles::SNOW | tiles::ICE)
}

/// `randGemTile`: 19 times out of 20, plain stone; the 1/20 goes to whichever gem this pocket
/// rolled. `gems` indexes [`tiles::GEM_WALLS`][gw]'s six slots by the same 0-5 order.
///
/// [gw]: super::tiles::walls::GEM_WALLS
fn rand_gem_tile(gems: [bool; 6], rand: &mut UnifiedRandom) -> u16 {
    const GEM_TILES: [u16; 6] = [
        tiles::AMETHYST,
        tiles::TOPAZ,
        tiles::SAPPHIRE,
        tiles::EMERALD,
        tiles::RUBY,
        tiles::DIAMOND,
    ];
    if rand.next_max(20) != 0 {
        return tiles::STONE;
    }
    GEM_TILES[rand_gem(gems, rand)]
}

/// `randGem`: rolls until it lands on one of the gems this pocket actually contains.
fn rand_gem(gems: [bool; 6], rand: &mut UnifiedRandom) -> usize {
    loop {
        let i = rand.next_max(6) as usize;
        if gems[i] {
            return i;
        }
    }
}

/// `Spread.Gem`, transcribed: wave flood-fill from `(x, y)`. A solid or walled tile gets its own
/// (and its four neighbours') gemmable type recolored; an open tile gets a gem wall instead and
/// joins the next wave.
///
/// **One deliberate deviation from the literal port, added alongside the siting fix below.**
/// Vanilla's wave has no size cap of its own — it relies on its own cave topology (small, mostly
/// enclosed pockets) to bound itself naturally, stopping wherever it meets solid or already-walled
/// rock. `structures::caves()` doesn't produce that topology: it carves one large interconnected
/// tunnel network, open and unwalled throughout its interior (correctly, since real caves in
/// vanilla are unwalled at this point in the pipeline too — see `caves()`'s own fix). A literal
/// port of this wave, run against that network, would not stop at a pocket's edge because there
/// often isn't one nearby; it would paint gem wall an unbounded distance down whatever tunnel it
/// started in. Capped at the same 300 tiles the siting check below used to reject candidates
/// larger than — repurposed from "how big is too big to be a valid site" to "how big a footprint
/// this decoration paints," since in this topology the first no longer means anything but the
/// second still does.
fn spread_gem(
    world: &mut super::super::World,
    x: i32,
    y: i32,
    gems: [bool; 6],
    rand: &mut UnifiedRandom,
) {
    const SPREAD_CAP: usize = 300;
    let mut seen: HashSet<(i32, i32)> = HashSet::new();
    let mut wave = vec![(x, y)];

    while !wave.is_empty() {
        if seen.len() >= SPREAD_CAP {
            break;
        }
        let this_wave = std::mem::take(&mut wave);
        for (cx, cy) in this_wave {
            if seen.len() >= SPREAD_CAP {
                break;
            }
            if cx < 1 || cx >= world.width() - 1 || cy < 1 || cy >= world.height() - 1 {
                continue;
            }
            if !seen.insert((cx, cy)) {
                continue;
            }
            let tile = world.tile(cx, cy);
            // `is_active()` first: `tile_solid::solid` is a pure lookup by tile *type*, and an
            // inactive tile's leftover `block` id (0, dirt's own id) reads as solid if that check
            // runs alone — the same ordering bug `place_object.rs` was fixed for earlier.
            let solid_or_walled =
                (tile.is_active() && tile_solid::solid(tile.block)) || tile.wall != 0;
            if solid_or_walled {
                if tile.is_active() {
                    for (px, py) in [
                        (cx, cy),
                        (cx - 1, cy),
                        (cx + 1, cy),
                        (cx, cy - 1),
                        (cx, cy + 1),
                    ] {
                        let mut t = world.tile(px, py);
                        if t.is_active() && gemmable(t.block) {
                            t.block = rand_gem_tile(gems, rand);
                            world.set_tile(px, py, t);
                        }
                    }
                }
                continue;
            }
            let mut t = tile;
            t.wall = walls::GEM_WALLS[rand_gem(gems, rand)];
            world.set_tile(cx, cy, t);
            // `Spread.Gem`'s own open-tile branch (`WorldGen.cs:3589-3592`): once in a while, an
            // inactive tile in the pocket's own open interior gets a genuinely exposed gem tile
            // instead of staying bare wall — the pocket's real, findable loot, not just a colored
            // backdrop. `PlaceTile`'s dedicated `num == 178` dispatch (`WorldGen.cs:60190-60200`)
            // writes `frameX = style * 18` (the species — `KillTile`'s drop table reads this back
            // as `frameX / 18`) and `frameY = genRand.Next(3) * 18` (a cosmetic variant); a second,
            // independent `randGem()` roll picks the style here, separate from the wall's own.
            if !world.tile(cx, cy).is_active() && rand.next_max(2) == 0 {
                let style = rand_gem(gems, rand);
                let mut gem = world.tile(cx, cy);
                gem.block = GEM_TILE;
                gem.frame_x = style as i16 * 18;
                gem.frame_y = rand.next_max(3) as i16 * 18;
                gem.flags.set(TileFlags::ACTIVE, true);
                world.set_tile(cx, cy, gem);
            }
            for n in [(cx - 1, cy), (cx + 1, cy), (cx, cy - 1), (cx, cy + 1)] {
                if !seen.contains(&n) {
                    wave.push(n);
                }
            }
        }
    }
}

/// The `GemCaves` pass: scatter gem-lined pockets through the rock layer.
///
/// Returns how many were placed.
pub fn scatter(
    world: &mut super::super::World,
    layout: &Layout,
    rand: &mut UnifiedRandom,
) -> usize {
    // The search bands below are `200..width-200` and `layout.rock+30..height-230`. Real,
    // full-size worlds always clear both by a wide margin, but the small synthetic worlds several
    // unrelated tests build (to keep persistence/gameplay tests fast) do not — the same shape of
    // guard `oasis.rs::scatter` needed for its own search bands. Skip rather than let
    // `next_range` panic on an inverted or empty range.
    if layout.width <= 400 || world.height() <= layout.rock + 260 {
        return 0;
    }

    let attempts = ((layout.width as f64) * 0.003) as i32;
    let mut placed = 0usize;

    for _ in 0..attempts {
        let mut tries = 0;
        let mut x = rand.next_range(200, layout.width - 200);
        let mut y = rand.next_range(layout.rock + 30, world.height() - 230);
        let mut found = cave_flood::count(world, x, y, 300, false, false);
        while (found.tiles < 50 || found.lava > 0 || found.ice > 0) && tries < 1000 {
            tries += 1;
            x = rand.next_range(200, layout.width - 200);
            y = rand.next_range(layout.rock + 30, world.height() - 230);
            found = cave_flood::count(world, x, y, 300, false, false);
        }
        if tries < 1000 {
            // `gemCave`: always one random gem, then each of the other five independently has a
            // 1-in-6 chance of also being included in this pocket's palette.
            let mut gems = [false; 6];
            gems[rand.next_max(6) as usize] = true;
            for g in gems.iter_mut() {
                if rand.next_max(6) == 0 {
                    *g = true;
                }
            }
            spread_gem(world, x, y, gems, rand);
            placed += 1;
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::World;
    use terrustia_proto::Tile;

    fn stone_block(width: i32, height: i32, rock: i32) -> (World, Layout) {
        let mut world = World::empty(width, height, "gem-caves");
        for x in 0..width {
            for y in 0..height {
                world.set_tile(x, y, Tile::block(tiles::STONE));
            }
        }
        let mut rand = UnifiedRandom::new(1);
        let mut layout = Layout::plan(width, height, &mut rand);
        layout.rock = rock;
        (world, layout)
    }

    /// `Spread.Gem`'s own open-tile branch (`WorldGen.cs:3589-3592`) was never transcribed — a
    /// gem cave's open interior only ever got wall paint, never the exposed gem tile itself that
    /// is the pocket's own real, findable loot. Fails on the pre-fix code (no tile 178 anywhere).
    #[test]
    fn spread_gem_places_real_exposed_gem_tiles_in_the_open_interior() {
        let (mut world, _layout) = stone_block(200, 200, 100);
        for x in 90..110 {
            for y in 90..110 {
                world.set_tile(x, y, Tile::AIR);
            }
        }
        let mut rand = UnifiedRandom::new(7);
        // Every gem in this pocket's palette, maximizing how much of the 400-tile room the
        // exposed-gem roll actually gets to run against.
        let gems = [true; 6];
        spread_gem(&mut world, 100, 100, gems, &mut rand);

        let gem_tiles: Vec<(i32, i32)> = (90..110)
            .flat_map(|x| (90..110).map(move |y| (x, y)))
            .filter(|&(x, y)| world.tile(x, y).is_active() && world.tile(x, y).block == GEM_TILE)
            .collect();
        assert!(
            !gem_tiles.is_empty(),
            "expected at least one real exposed gem tile in a 400-tile open pocket"
        );
        for (x, y) in gem_tiles {
            let t = world.tile(x, y);
            assert!(
                (0..6).contains(&(t.frame_x / 18)) && t.frame_x % 18 == 0,
                "gem at ({x},{y}) has an invalid species frame_x {}",
                t.frame_x
            );
        }
    }

    #[test]
    fn a_hollow_pocket_in_the_rock_layer_gets_gem_walls() {
        let (mut world, layout) = stone_block(1200, 900, 300);
        // A pocket sized inside GemCaves' own 50-299 window, well below the rock layer.
        for x in 595..610 {
            for y in 595..605 {
                world.set_tile(x, y, Tile::AIR);
            }
        }
        let mut rand = UnifiedRandom::new(42);
        let placed = scatter(&mut world, &layout, &mut rand);
        assert!(
            placed > 0,
            "a well-formed pocket in the rock layer should take a gem cave"
        );

        let walled = (0..world.width())
            .flat_map(|x| (0..world.height()).map(move |y| (x, y)))
            .filter(|&(x, y)| tiles::walls::GEM_WALLS.contains(&world.tile(x, y).wall))
            .count();
        assert!(
            walled > 0,
            "a placed gem cave should leave real gem-walled tiles behind"
        );
    }

    #[test]
    fn no_gem_cave_forms_where_every_pocket_is_too_small() {
        // Solid rock everywhere, no hollows at all — every candidate site fails countTiles' own
        // >=50 floor, so nothing should be placed.
        let (mut world, layout) = stone_block(600, 500, 200);
        let mut rand = UnifiedRandom::new(7);
        assert_eq!(scatter(&mut world, &layout, &mut rand), 0);
    }

    /// A real regression: the small synthetic worlds several unrelated tests build via the full
    /// `build()` pipeline are smaller than this pass's own search bands assume, and
    /// `UnifiedRandom::next_range` panics on an inverted or empty range rather than returning
    /// something — `world.height() - 230` going below `layout.rock + 30` (or `layout.width - 200`
    /// below `200`) took down `world::wld_save`/`world::world::flag_tests` tests that never
    /// touch gem caves directly, just by calling `build()` on a small world. Fails on the pre-fix
    /// code (panics rather than returning `0`).
    #[test]
    fn a_world_too_small_for_the_search_bands_does_not_panic() {
        // Width 1000 keeps `attempts` (`width * 0.003`) at 3, not 0 — a width small enough to
        // zero out `attempts` would skip the loop entirely and never reach the panicking call,
        // proving nothing. `height=200, rock=50` makes `layout.rock + 30` (80) exceed
        // `world.height() - 230` (-30), the actual inverted-range shape that panicked.
        let (mut world, layout) = stone_block(1000, 200, 50);
        let mut rand = UnifiedRandom::new(1);
        assert_eq!(scatter(&mut world, &layout, &mut rand), 0);
    }

    /// The actual defect this module shipped with: `structures::caves()` produces one large,
    /// genuinely-connected tunnel network rather than vanilla's small isolated pockets — so a
    /// pocket this large is exactly what a real generated world's candidate sites look like, and
    /// the old `found.tiles >= 300` rejection turned that into "reject everywhere." Fails on the
    /// pre-fix code (restoring the `>= 300` check makes `placed` come back `0`).
    #[test]
    fn a_pocket_far_larger_than_the_old_upper_bound_is_still_accepted() {
        let (mut world, layout) = stone_block(1200, 900, 300);
        // A long, wide corridor — thousands of open tiles, the shape a real carved tunnel network
        // actually produces, not a small enclosed room.
        for x in 100..1100 {
            for y in 595..605 {
                world.set_tile(x, y, Tile::AIR);
            }
        }
        let mut rand = UnifiedRandom::new(42);
        let placed = scatter(&mut world, &layout, &mut rand);
        assert!(
            placed > 0,
            "a large, well-connected, otherwise-valid pocket must not be rejected for its size \
             alone — that rejection is what made every real generated world place zero"
        );
    }

    /// The *second* defect the pocket-size fix alone didn't cover: `rockCount == 0` also rejects a
    /// candidate whenever the 300-tile search budget is spent entirely inside open space without
    /// ever touching a stone tile — which is exactly what happens deep inside a large open network,
    /// since `cave_flood::count`'s stack order keeps preferring an unvisited "one step further"
    /// neighbour and so drives the fill straight onward rather than spreading out toward a wall.
    ///
    /// `scatter` itself picks the candidate `(x, y)`, sampled anywhere in
    /// `[layout.rock + 30, height - 230)`, so this test doesn't get to choose the seed point
    /// directly — instead it carves every row from the rock layer down to the bottom of the world,
    /// across the whole width. That way *every* possible sampled `y` sits at least 300 rows above
    /// either a wall or the world's own edge (both routes to the search cap), so the flood's whole
    /// budget is spent in open space and `rockCount` stays `0` on a real, well-formed candidate
    /// regardless of which one gets picked. Fails on the pre-fix code (restoring
    /// `|| found.rock == 0` makes `placed` come back `0`).
    #[test]
    fn a_pocket_whose_search_budget_never_reaches_a_wall_is_still_accepted() {
        let (mut world, layout) = stone_block(1200, 900, 300);
        for x in 0..1200 {
            for y in 300..900 {
                world.set_tile(x, y, Tile::AIR);
            }
        }
        let mut rand = UnifiedRandom::new(3);
        let placed = scatter(&mut world, &layout, &mut rand);
        assert!(
            placed > 0,
            "a real, well-formed pocket must not be rejected just because its own search budget \
             ran out before reaching a wall — that rejection is what left rockCount==0 on almost \
             every candidate in a real generated world"
        );
    }

    /// [`spread_gem`]'s own cap: painting an unbounded distance down a long open corridor would be
    /// the same bug relocated from siting to decoration. Confirms the footprint actually stays
    /// bounded rather than consuming the whole corridor this test deliberately makes very long.
    #[test]
    fn spread_gem_does_not_paint_an_unbounded_distance_down_a_long_corridor() {
        let (mut world, layout) = stone_block(3600, 900, 300);
        for x in 100..3500 {
            for y in 595..605 {
                world.set_tile(x, y, Tile::AIR);
            }
        }
        let mut rand = UnifiedRandom::new(11);
        let placed = scatter(&mut world, &layout, &mut rand);
        assert!(
            placed > 0,
            "expected at least one gem cave in a 3400-tile-long corridor"
        );

        let walled = (0..world.width())
            .flat_map(|x| (0..world.height()).map(move |y| (x, y)))
            .filter(|&(x, y)| tiles::walls::GEM_WALLS.contains(&world.tile(x, y).wall))
            .count();
        assert!(
            walled > 0 && walled <= 300 * placed,
            "{walled} gem-walled tiles across {placed} pocket(s) — spread_gem's cap should keep \
             each pocket's footprint at or under 300 tiles, not paint the whole corridor"
        );
    }
}
