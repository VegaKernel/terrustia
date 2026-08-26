//! Micro-biomes: `MicroBiomes` (the driving pass, `WorldGen.cs:21069-21422`) + `Marble`/`Granite`
//! (`:12812-12983`), which between them call out to the 15 `MicroBiome`-derived classes in
//! `Terraria.GameContent.Biomes/` (`.scratch/decompiled/Terraria.GameContent.Biomes/`,
//! confirmed by grepping every `class X : MicroBiome` in that directory — exactly 15, matching
//! `plan.md`'s own sizing table).
//!
//! **Of the 15, one — `CaveHouseBiome` — was already done** (`underground_cabins.rs`, landed
//! earlier this session). **This module lands 6 more**, real and tested: [`thin_ice`],
//! [`corruption_pit`], [`spike_pit`], [`honey_patch`], [`campsite`], and a shared ellipse-blob
//! painter used for both `marble` and `granite`. That is 7 of 15 done in this module, 8 of 15
//! overall.
//!
//! **A real, substantial correction to `plan.md`'s own "correction to the correction."** That
//! section, written before this module existed, said the DSL dependency found in `CaveHouseBiome`
//! was "a genuinely narrow slice, five or six specific operations, not the whole composable
//! pipeline." Reading all 15 classes for this module disproves that: every single one of them —
//! not just `CaveHouseBiome` — leans on `WorldUtils.Gen`/`Shapes.{Circle,Slime,Mound,Rectangle}`/
//! `Modifiers.{Blotches,Offset,RadialDither,Checkerboard,RectangleMask,Expand,SkipTiles,SkipWalls,
//! OnlyTiles,OnlyWalls,IsTouchingAir,IsTouching,NotTouching}`/`Actions.{SetTile,ClearTile,
//! PlaceWall,SetLiquid,Scanner,TileScanner,SetFrames}`/`ShapeData.Subtract`/`ModShapes.{All,
//! InnerOutline}` — the *blotchy, dithered, noise-textured edge* is not an optional decorative
//! detail on top of these biomes, it is most of what makes each one look like its own biome rather
//! than a geometric primitive. Faithfully porting that would mean building a real subset of the
//! `Shapes`/`Modifiers`/`Actions` pipeline after all — exactly the ~5,253-line framework this
//! project's own sizing pass twice already concluded was not needed. It is needed here, and this
//! module does not build it: every shape below is a plain circle, rectangle or ellipse from this
//! generator's own tile API, matching the project's stated preference for a narrow, purpose-built
//! implementation over porting the general framework — the same call `cave_flood.rs` made for its
//! own flood-fill, `underground_cabins.rs` for its own room-finder, and every Tier 2 item so far for
//! its own siting. What's lost, named plainly rather than left implicit: rough/organic edges (real
//! vanilla micro-biomes rarely have a perfectly round or straight boundary; these do), and every
//! shading/dithering pass vanilla layers on top of the base shape.
//!
//! **The 8 not done, and why**, matching this session's standing disclosure practice:
//! * `DeadMansChestBiome` (626 lines) — by far the largest of the 15, needs a pre-existing
//!   trappable-chest mechanism this generator does not have and `DitherSnake`/`DitherSnakePass`
//!   (a further ~500 lines in the same namespace) for its own tunnel dressing. Out of reach this
//!   session.
//! * `DesertBiome` (72 lines) — a further correction, found reading it for this module: those 72
//!   lines are mostly a *dispatcher* to a whole separate `Terraria.GameContent.Biomes.Desert`
//!   sub-namespace (`SandMound`, `ChambersEntrance`, `AnthillEntrance`, `LarvaHoleEntrance`,
//!   `PitEntrance`, `DesertHive`) that vanilla's own real "Underground Desert" generation lives in
//!   — none of those classes are counted in `plan.md`'s own 4,240-line/15-class total, and porting
//!   them is realistically its own Tier-2-sized item, not a detail inside this one. Out of scope.
//! * `DunesBiome` (162 lines) — `pyramids.rs`'s own Done row already disclosed sourcing pyramid
//!   sites from `layout.desert` directly rather than this class; still genuinely not ported.
//! * `MahoganyTreeBiome` (94 lines) — a second, separate tree-growing subsystem distinct from
//!   `living_trees.rs`'s own `GrowLivingTree` port (confirmed by reading it: different tile ids,
//!   383/384 vs Living Wood, and its own `ShapeBranch`/`ShapeRoot` growth shapes, sited specifically
//!   in the jungle with a real jungle-chest reward). Comparable in scope to `living_trees.rs`
//!   itself; not attempted here.
//! * `EnchantedSwordBiome` (113 lines) — a below-ground clearing dug up toward the surface with a
//!   dungeon-adjacency exclusion and its own `Shapes.Slime`/`Shapes.Mound` pair; moderately complex,
//!   deferred for budget rather than found intractable.
//! * `MiningExplosivesBiome` (85 lines) — needs a wandering-tunnel shape (`ShapeRunner`) and
//!   `WorldUtils.WireLine` to connect a pressure plate to an explosive; deferred for budget.
//! * `HiveBiome` (425 lines) — the single largest of the remaining classes, its own bespoke
//!   pocket/stalactite carving system, driven by a separate `Beehives` pass (`WorldGen.cs:16017`)
//!   that scatters wild bee-hive pockets across the whole world at mid-cavern depth, not just the
//!   jungle. **Not the same thing** `structures::hive` already builds — that function is a single
//!   jungle-restricted hive with the Queen Bee's larva, transcribed independently earlier this
//!   session and disclosed there as such; this class's real job is a *different*, larger feature.
//!   Out of reach this session.
//! * `CaveHouseBiome` — **already done**, see `underground_cabins.rs`'s own Done row.

use terrustia_proto::{Tile, tile_solid};

use super::layout::{Evil, Layout};
use super::place_object::place_object;
use super::rand::UnifiedRandom;
use super::structure_map::{Rect, StructureMap};
use super::tiles;
use crate::world::World;

/// What a call to [`scatter`] placed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    pub thin_ice: usize,
    pub corruption_pits: usize,
    pub spike_pits: usize,
    pub honey_patches: usize,
    pub campsites: usize,
    pub marble: usize,
    pub granite: usize,
}

/// The `MicroBiomes` + `Marble` + `Granite` passes, merged into one call — see the module doc for
/// exactly which of the 15 real `MicroBiome` classes this covers.
pub fn scatter(
    world: &mut World,
    layout: &Layout,
    structures: &mut StructureMap,
    rand: &mut UnifiedRandom,
) -> Report {
    let mut report = Report::default();
    if layout.width < 900 || layout.height < 400 {
        return report;
    }

    // `ThinIcePatchCount`: Min 3, Max 5 (`Configuration.json`).
    let count = rand.next_range(3, 6);
    for _ in 0..count {
        let mut tries = 0;
        while tries < 200 {
            tries += 1;
            if layout.snow.width() <= 0 {
                break;
            }
            let x = rand.next_range(layout.snow.from, layout.snow.to.max(layout.snow.from + 1));
            let y = rand.next_range(layout.surface + 20, layout.surface + 220);
            if thin_ice(world, structures, rand, x, y) {
                report.thin_ice += 1;
                break;
            }
        }
    }

    // `CorruptionPitCount`: Min 1, Max 2. Ebonstone-only in real vanilla too — see `corruption_pit`.
    if layout.evil == Evil::Corruption {
        let count = rand.next_range(1, 3);
        for _ in 0..count {
            let mut tries = 0;
            while tries < 200 {
                tries += 1;
                let x = rand.next_range(
                    layout.evil_band.from,
                    layout.evil_band.to.max(layout.evil_band.from + 1),
                );
                let y = rand.next_range(layout.rock + 30, layout.underworld - 100);
                if corruption_pit(world, structures, rand, x, y) {
                    report.corruption_pits += 1;
                    break;
                }
            }
        }
    }

    // `WorldGen.cs:24355-24368`: 3 on a small world, `+ genRand.Next(2)`.
    let count = rand.next_range(3, 5);
    for _ in 0..count {
        let mut tries = 0;
        while tries < 200 {
            tries += 1;
            let x = rand.next_range(
                200.min(layout.width / 2),
                layout.width - 200.min(layout.width / 2),
            );
            let y = rand.next_range(layout.rock + 30, layout.height - 230);
            if spike_pit(world, structures, rand, x, y) {
                report.spike_pits += 1;
                break;
            }
        }
    }

    // Real vanilla sites this against a hive this module does not build (see the module doc) —
    // sited directly against `layout.jungle` instead, since the class's own site check already
    // requires nearby jungle grass.
    let count = rand.next_range(4, 9);
    for _ in 0..count {
        let mut tries = 0;
        while tries < 200 {
            tries += 1;
            if layout.jungle.width() <= 0 {
                break;
            }
            let x = rand.next_range(
                layout.jungle.from + 10,
                (layout.jungle.to - 10).max(layout.jungle.from + 11),
            );
            let y = rand.next_range(layout.rock, layout.underworld - 60);
            if honey_patch(world, structures, rand, x, y) {
                report.honey_patches += 1;
                break;
            }
        }
    }

    // `CampsiteCount`: Min 6, Max 11.
    let count = rand.next_range(6, 12);
    for _ in 0..count {
        let mut tries = 0;
        while tries < 200 {
            tries += 1;
            let x = rand.next_range(60, (layout.width - 60).max(61));
            if campsite(world, layout, structures, rand, x) {
                report.campsites += 1;
                break;
            }
        }
    }

    // `Marble`/`Granite` `Count`: Min 4, Max 8 each. Real vanilla sites one per horizontal band of
    // the world at cavern depth, avoiding a narrow band around dead centre on an ordinary world —
    // transcribed narrower, without the per-band bookkeeping: `count` independent random sites.
    let marble_count = rand.next_range(4, 9);
    for _ in 0..marble_count {
        let x = rand.next_range(100, (layout.width - 100).max(101));
        let mid = layout.width / 2;
        if x > mid - (layout.width / 20) && x < mid + (layout.width / 20) {
            continue;
        }
        let y = rand.next_range(layout.rock + 20, layout.height - 200);
        paint_ellipse_biome(
            world,
            structures,
            rand,
            x,
            y,
            tiles::MARBLE,
            tiles::walls::MARBLE,
        );
        report.marble += 1;
    }
    let granite_count = rand.next_range(4, 9);
    for _ in 0..granite_count {
        let x = rand.next_range(100, (layout.width - 100).max(101));
        let mid = layout.width / 2;
        if x > mid - (layout.width / 20) && x < mid + (layout.width / 20) {
            continue;
        }
        let y = rand.next_range(layout.rock + 20, layout.height - 200);
        paint_ellipse_biome(
            world,
            structures,
            rand,
            x,
            y,
            tiles::GRANITE,
            tiles::walls::GRANITE,
        );
        report.granite += 1;
    }

    report
}

/// `WorldUtils.Find(origin, Searches.Chain(Searches.Down(n), Conditions.IsSolid()))` and its
/// siblings — scan up to `max` tiles in one direction for the first solid, active tile. Re-derived
/// locally rather than shared, matching every other siting pass in this generator.
fn find_solid(world: &World, x: i32, y: i32, dx: i32, dy: i32, max: i32) -> Option<(i32, i32)> {
    let (mut cx, mut cy) = (x, y);
    for _ in 0..max {
        if !world.in_bounds(cx, cy) {
            return None;
        }
        let t = world.tile(cx, cy);
        if t.is_active() && tile_solid::solid(t.block) {
            return Some((cx, cy));
        }
        cx += dx;
        cy += dy;
    }
    None
}

/// Every tile within `radius` of `(cx, cy)` (a plain circle — see the module doc's "not the full
/// DSL" note for what this deliberately drops), visited in scanline order.
fn circle(cx: i32, cy: i32, radius: i32, mut f: impl FnMut(i32, i32)) {
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            if dx * dx + dy * dy <= radius * radius {
                f(cx + dx, cy + dy);
            }
        }
    }
}

/// `ThinIceBiome::Place` (`ThinIceBiome.cs:9-34`). Narrowed: vanilla's blotchy-edged, tapering
/// stack of circles becomes a plain, evenly-shrinking stack; the `SquareTileFrame`-driven liquid
/// clear is folded into the same pass instead of a second `WorldUtils.Gen` call.
fn thin_ice(
    world: &mut World,
    structures: &mut StructureMap,
    rand: &mut UnifiedRandom,
    x: i32,
    y: i32,
) -> bool {
    if !world.in_bounds(x - 25, y - 25) || !world.in_bounds(x + 25, y + 25) {
        return false;
    }
    let (mut dirt_stone, mut mud, mut snow, mut hive) = (0, 0, 0, 0);
    for dx in -25..25 {
        for dy in -25..25 {
            let t = world.tile(x + dx, y + dy);
            if !t.is_active() {
                continue;
            }
            match t.block {
                tiles::DIRT | tiles::STONE => dirt_stone += 1,
                tiles::MUD => mud += 1,
                tiles::SNOW => snow += 1,
                tiles::HIVE => hive += 1,
                _ => {}
            }
        }
    }
    if hive > 0 || snow <= mud || snow <= dirt_stone {
        return false;
    }

    let mut cy = y;
    let mut radius = rand.next_range(10, 15);
    while radius > 5 {
        let cx = x + rand.next_range(-5, 5);
        circle(cx, cy, radius, |px, py| {
            if !world.in_bounds(px, py) {
                return;
            }
            let t = world.tile(px, py);
            let melts = t.is_active()
                && matches!(
                    t.block,
                    tiles::SNOW | tiles::ICE | tiles::DIRT | tiles::STONE
                );
            let thawed = t.liquid > 0;
            if melts || thawed {
                let mut new_t = Tile::block(tiles::BREAKABLE_ICE);
                new_t.wall = t.wall;
                world.set_tile(px, py, new_t);
            }
        });
        cy += radius - 2;
        radius -= 1;
    }

    // Informational only — real vanilla's own `AddStructure`, not `AddProtectedStructure`; a thin
    // ice patch does not block a later placement, the same asymmetry `structure_map.rs`'s own doc
    // comment already flags for vanilla's `StructureMap` in general.
    structures.add_structure(Rect::new(x - 25, y - 25, 50, 50), 8);
    true
}

/// `CorruptionPitBiome::Place` (`CorruptionPitBiome.cs:11-55`). Ebonstone-only in real vanilla too
/// — its own site check requires nearby tile 25, which never occurs on a Crimson world, so it is
/// naturally Corruption-exclusive rather than something this port had to special-case.
fn corruption_pit(
    world: &mut World,
    structures: &mut StructureMap,
    rand: &mut UnifiedRandom,
    x: i32,
    y: i32,
) -> bool {
    let Some((ox, oy)) = find_solid(world, x, y, 0, 1, 100) else {
        return false;
    };
    // A real Ebonstone floor nearby, matching `IsTile(25).AreaAnd(8, 1)`.
    let mut ebonstone_row = false;
    for dx in -4..4 {
        if world.tile(ox + dx, oy).block == tiles::EBONSTONE && world.tile(ox + dx, oy).is_active()
        {
            ebonstone_row = true;
            break;
        }
    }
    if !ebonstone_row {
        return false;
    }

    let bounds = Rect::new(ox - 15, oy - 20, 30, 50);
    // Real vanilla's own `ValidTiles` here is not `GeneralPlacementTiles` — `CorruptionPitBiome`
    // rolls its own bool set (`CorruptionPitBiome.cs:9`, `CreateBoolSet(true, 21, 31, 26)`) that
    // only excludes a chest, a Shadow Orb and a Demon Altar, nothing else. A real bug caught by
    // this function's own test: using the shared default set instead refused every placement,
    // since `general_placement_tile` itself excludes Ebonstone (25) — the exact tile this site
    // check just required to already be there.
    if !structures.can_place_with(world, bounds, 2, |block| {
        !matches!(block, tiles::CHEST | tiles::SHADOW_ORB | tiles::DEMON_ALTAR)
    }) {
        return false;
    }

    // Three widening layers, matching vanilla's own real per-layer radius/offset progression —
    // outer Ebonstone shell, a Dirt ring, and a hollow core.
    for i in 0..6 {
        circle(ox, oy + 5 * i + 5, rand.next_range(10, 13) + i, |px, py| {
            let mut t = Tile::block(tiles::EBONSTONE);
            t.wall = tiles::walls::EBONSTONE;
            if world.in_bounds(px, py) {
                world.set_tile(px, py, t);
            }
        });
    }
    for i in 0..6 {
        circle(ox, oy + 2 * i + 18, rand.next_range(5, 8) + i, |px, py| {
            if world.in_bounds(px, py) {
                let wall = world.tile(px, py).wall;
                let mut t = Tile::block(tiles::DIRT);
                t.wall = wall;
                world.set_tile(px, py, t);
            }
        });
    }
    for i in 0..6 {
        let off = (7.5 * f64::from(i)) as i32 - 10;
        circle(ox, oy + off, rand.next_range(4, 6) + i / 2, |px, py| {
            if world.in_bounds(px, py) {
                let wall = world.tile(px, py).wall;
                let mut t = Tile::AIR;
                t.wall = wall;
                world.set_tile(px, py, t);
            }
        });
    }

    structures.add_protected_structure(bounds, 2);
    true
}

/// `SpikePitBiome::Place` (`SpikePitBiome.cs:8-57`). Vanilla's `Modifiers.Checkerboard`-placed
/// spike floor becomes a plain alternating-column placement of the same tile.
fn spike_pit(
    world: &mut World,
    structures: &mut StructureMap,
    rand: &mut UnifiedRandom,
    x: i32,
    y: i32,
) -> bool {
    let Some((ox, oy)) = find_solid(world, x, y, 0, 1, 100) else {
        return false;
    };
    let mut stone_row = false;
    for dx in -4..4 {
        let t = world.tile(ox + dx, oy);
        if t.is_active() && t.block == tiles::STONE {
            stone_row = true;
            break;
        }
    }
    if !stone_row {
        return false;
    }

    let bounds = Rect::new(ox - 15, oy - 20, 30, 40);
    if !structures.can_place(world, bounds, 2) {
        return false;
    }

    for i in 0..4 {
        circle(ox, oy + 5 * i + 5, rand.next_range(8, 11) + i, |px, py| {
            if world.in_bounds(px, py) {
                world.set_tile(px, py, Tile::block(tiles::STONE));
            }
        });
    }
    for i in 0..4 {
        circle(
            ox,
            oy + off_spike(i),
            rand.next_range(4, 6) + i / 2,
            |px, py| {
                if world.in_bounds(px, py) {
                    world.set_tile(px, py, Tile::AIR);
                }
            },
        );
    }
    // The spike floor: alternating columns along the cleared hollow's bottom edge.
    let hollow_bottom = oy + off_spike(3) + rand.next_range(4, 6) + 1;
    for (i, px) in (ox - 12..=ox + 12).enumerate() {
        if i % 2 != 0 {
            continue;
        }
        if world.tile(px, hollow_bottom).is_active()
            && !world.tile(px, hollow_bottom - 1).is_active()
        {
            world.set_tile(px, hollow_bottom - 1, Tile::block(tiles::SPIKES));
        }
    }

    structures.add_protected_structure(bounds, 2);
    true
}

fn off_spike(i: i32) -> i32 {
    (7.5 * f64::from(i)) as i32 - 10
}

/// `HoneyPatchBiome::Place` (`HoneyPatchBiome.cs:8-48`). Sited directly against `layout.jungle`
/// rather than requiring a prior `HiveBiome` placement this module does not build — see the module
/// doc.
fn honey_patch(
    world: &mut World,
    structures: &mut StructureMap,
    _rand: &mut UnifiedRandom,
    x: i32,
    y: i32,
) -> bool {
    let Some((ox, mut oy)) = find_solid(world, x, y, 0, 1, 80) else {
        return false;
    };
    oy += 2;

    let (mut solid, mut jungly) = (0, 0);
    circle(ox, oy, 15, |px, py| {
        if !world.in_bounds(px, py) {
            return;
        }
        let t = world.tile(px, py);
        if t.is_active() && tile_solid::solid(t.block) {
            solid += 1;
            if matches!(t.block, tiles::JUNGLE_GRASS | tiles::MUD) {
                jungly += 1;
            }
        }
    });
    if solid == 0 || f64::from(jungly) / f64::from(solid) < 0.75 {
        return false;
    }

    let bounds = Rect::new(ox - 8, oy - 8, 16, 16);
    if !structures.can_place(world, bounds, 0) {
        return false;
    }
    if oy >= layout_underworld_guard(world) {
        return false;
    }

    // A solid honeycomb cap...
    circle(ox, oy, 8, |px, py| {
        if !world.in_bounds(px, py) {
            return;
        }
        let t = world.tile(px, py);
        if t.is_active() && tile_solid::solid(t.block) {
            world.set_tile(px, py, Tile::block(tiles::HONEY_BLOCK));
        }
    });
    // ...hollowed underneath and filled with honey liquid.
    circle(ox, oy + 3, 4, |px, py| {
        if !world.in_bounds(px, py) || py <= oy {
            return;
        }
        let t = world.tile(px, py);
        if t.is_active() && tile_solid::solid(t.block) {
            let mut liquid = Tile::AIR;
            liquid.liquid = 255;
            liquid.liquid_kind = terrustia_proto::Liquid::Honey;
            world.set_tile(px, py, liquid);
        }
    });
    structures.add_protected_structure(bounds, 0);
    true
}

/// A generous fixed depth guard in place of vanilla's `Main.UnderworldLayer - 30` check
/// (`HoneyPatchBiome`'s own `TooCloseToImportantLocations`) — this module has no direct access to
/// that constant from here, so the caller's own `layout.underworld` bound (already used to pick
/// `y` in the first place) already keeps a honey patch out of the underworld; this is a second,
/// cheap belt-and-suspenders check against `World::height` directly.
fn layout_underworld_guard(world: &World) -> i32 {
    world.height() - 60
}

/// `CampsiteBiome::Place` (`CampsiteBiome.cs:9-116`), narrowed: vanilla's `IsGroupSolid`-driven
/// stone-smoothing and its dungeon/container/altar exclusion sweep are dropped in favour of the
/// same `StructureMap::can_place` check every other siting pass here already uses; the wall
/// material adapts to `Layout::surface_biome` directly rather than scanning nearby tiles for it,
/// the same "use what `Layout` already knows" substitution `jungle_shrines.rs`/
/// `floating_islands.rs` already made.
fn campsite(
    world: &mut World,
    layout: &Layout,
    structures: &mut StructureMap,
    rand: &mut UnifiedRandom,
    x: i32,
) -> bool {
    let Some((ox, oy)) = find_solid(
        world,
        x,
        layout.surface,
        0,
        1,
        layout.height - layout.surface,
    ) else {
        return false;
    };
    let oy = oy - 1;
    let half = rand.next_range(6, 10);

    let bounds = Rect::new(ox - half, oy - half, half * 2, half * 2);
    if !structures.can_place(world, bounds, 4) {
        return false;
    }

    let wall = match layout.surface_biome(ox) {
        Some(super::layout::Surface::Desert) => tiles::walls::SANDSTONE,
        Some(super::layout::Surface::Snow) => tiles::walls::SNOW,
        Some(super::layout::Surface::Jungle) => tiles::walls::JUNGLE,
        _ => tiles::walls::CAVE,
    };

    // Headroom above the campsite's own floor — never touches `oy` (already open air) or below,
    // so the solid ground a placed object needs beneath it always survives. A real, found bug: the
    // first version of this check had the comparison backwards (`py < oy`, clearing the floor
    // *and* everything below it instead of the air above), which undermined the campfire's own
    // footprint and made `place_object` refuse it every time — caught by this function's own test.
    circle(ox, oy, half, |px, py| {
        if py > oy || !world.in_bounds(px, py) {
            return;
        }
        let mut t = Tile::AIR;
        t.wall = wall;
        world.set_tile(px, py, t);
    });

    place_object(world, ox, oy, tiles::CAMPFIRE, 0, -1);
    if rand.next_bool() {
        place_object(world, ox - half + 2, oy, tiles::GOLD_COIN_PILE, 0, -1);
    }

    structures.add_protected_structure(bounds, 4);
    true
}

/// The core shape both `marble` and `granite` share: an ellipse with a gentle vertical drift down
/// its length, painted directly over whatever solid stone is already there (preserving any ore —
/// `TileID.Sets.Ore` — the same way vanilla's own `PlaceSlab` does with `tile.ResetToType`).
///
/// Real vanilla `MarbleBiome::Place` (`MarbleBiome.cs:180-253`) works over a 3x3-tile "slab" grid
/// with its own diagonal corner-smoothing sub-states (`SlabStates`); `GraniteBiome::Place`
/// (`GraniteBiome.cs`) is a completely different, much larger algorithm — a 200x200-cell
/// pressure/flow cellular automaton run for up to 300 iterations. Neither is ported here: this
/// paints both biomes with the same simplified ellipse, `granite` genuinely is not a transcription
/// of `GraniteBiome`'s real algorithm and is disclosed as such in the module doc — a structural
/// stand-in, not a faithful port, the same class of deliberate substitution `pyramids.rs` already
/// made for `DunesBiome`.
fn paint_ellipse_biome(
    world: &mut World,
    structures: &mut StructureMap,
    rand: &mut UnifiedRandom,
    cx: i32,
    cy: i32,
    material: u16,
    wall: u16,
) -> Rect {
    let half_w = (rand.next_range(80, 150) / 2).max(10);
    let half_h = (rand.next_range(40, 60) / 2).max(6);
    let drift_a = rand.next_double() * 2.0 - 1.0;
    let drift_b = rand.next_double() * 2.0 - 1.0;
    let drift_c = rand.next_double() * 2.0 - 1.0;

    let (mut min_x, mut max_x, mut min_y, mut max_y) = (cx, cx, cy, cy);
    let mut y_off = 0.0f64;
    for m in -half_w..=half_w {
        let t = (m + half_w) as f64 / (half_w as f64 * 2.0).max(1.0);
        let inner = 1.0 - (m as f64 / half_w as f64).powi(2);
        let local_half_h = (f64::from(half_h) * inner.max(0.0).sqrt()) as i32;
        let step = if t < 0.5 {
            drift_a + (drift_b - drift_a) * (t * 2.0)
        } else {
            drift_b + (drift_c - drift_b) * ((t - 0.5) * 2.0)
        };
        y_off += step * 0.3;
        let row_cy = cy + y_off as i32;

        for n in -local_half_h..=local_half_h {
            let (x, y) = (cx + m, row_cy + n);
            if !world.in_bounds(x, y) {
                continue;
            }
            let cur = world.tile(x, y);
            if !cur.is_active() || !tile_solid::solid(cur.block) {
                continue;
            }
            let mut new_t = cur;
            if !is_ore(cur.block) {
                new_t.block = material;
            }
            new_t.wall = wall;
            world.set_tile(x, y, new_t);
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }

    let bounds = Rect::new(min_x, min_y, max_x - min_x, max_y - min_y);
    // Informational only, matching real vanilla: neither `MarbleBiome` nor (by disclosed
    // substitution) this `granite` stand-in calls `StructureMap::CanPlace` before painting.
    structures.add_structure(bounds, 8);
    bounds
}

/// `TileID.Sets.Ore`, narrowed to the ids this generator's own tile table names.
fn is_ore(block: u16) -> bool {
    matches!(
        block,
        tiles::COPPER
            | tiles::IRON
            | tiles::SILVER
            | tiles::GOLD
            | tiles::DEMONITE
            | tiles::CRIMTANE
            | tiles::SAPPHIRE
            | tiles::RUBY
            | tiles::EMERALD
            | tiles::TOPAZ
            | tiles::AMETHYST
            | tiles::DIAMOND
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stone_world(width: i32, height: i32, rock: i32) -> (World, Layout) {
        let mut world = World::empty(width, height, "micro-biomes");
        let mut rand = UnifiedRandom::new(1);
        let mut layout = Layout::plan(width, height, &mut rand);
        layout.rock = rock;
        layout.underworld = height - 100;
        for x in 0..width {
            for y in rock..(height - 50) {
                world.set_tile(x, y, Tile::block(tiles::STONE));
            }
        }
        (world, layout)
    }

    #[test]
    fn thin_ice_needs_a_real_snow_majority() {
        let (mut world, _layout) = stone_world(1600, 700, 200);
        // No snow anywhere: must refuse.
        let mut structures = StructureMap::new();
        let mut rand = UnifiedRandom::new(2);
        assert!(!thin_ice(&mut world, &mut structures, &mut rand, 200, 220));

        for x in 150..250 {
            for y in 200..260 {
                world.set_tile(x, y, Tile::block(tiles::SNOW));
            }
        }
        assert!(thin_ice(&mut world, &mut structures, &mut rand, 200, 220));
        let mut ice = 0;
        for x in 150..250 {
            for y in 200..260 {
                if world.tile(x, y).block == tiles::BREAKABLE_ICE {
                    ice += 1;
                }
            }
        }
        assert!(
            ice > 0,
            "no breakable ice was placed over a real snow patch"
        );
    }

    #[test]
    fn corruption_pit_needs_real_ebonstone_and_carves_a_hollow_core() {
        let (mut world, _layout) = stone_world(1600, 700, 200);
        let mut structures = StructureMap::new();
        let mut rand = UnifiedRandom::new(3);
        // Origin above the solid ground, matching a real siting call: `find_solid` walks down to
        // row 200 (the first solid tile), so that is where the Ebonstone floor has to be.
        // Plain stone: no Ebonstone anywhere, must refuse.
        assert!(!corruption_pit(
            &mut world,
            &mut structures,
            &mut rand,
            400,
            190
        ));

        for x in 380..420 {
            world.set_tile(x, 200, Tile::block(tiles::EBONSTONE));
        }
        assert!(corruption_pit(
            &mut world,
            &mut structures,
            &mut rand,
            400,
            190
        ));
        let mut ebonstone = 0;
        let mut hollow = 0;
        for x in 350..450 {
            for y in 200..320 {
                let t = world.tile(x, y);
                if t.block == tiles::EBONSTONE && t.is_active() {
                    ebonstone += 1;
                }
                if !t.is_active() && t.wall == tiles::walls::EBONSTONE {
                    hollow += 1;
                }
            }
        }
        assert!(ebonstone > 0, "no Ebonstone shell was carved");
        assert!(hollow > 0, "no hollow core was carved");
    }

    #[test]
    fn spike_pit_carves_a_hollow_with_spikes_on_the_floor() {
        let (mut world, _layout) = stone_world(1600, 700, 200);
        let mut structures = StructureMap::new();
        let mut rand = UnifiedRandom::new(4);
        assert!(spike_pit(&mut world, &mut structures, &mut rand, 400, 250));
        let mut spikes = 0;
        for x in 350..450 {
            for y in 200..320 {
                if world.tile(x, y).block == tiles::SPIKES {
                    spikes += 1;
                }
            }
        }
        assert!(spikes > 0, "no spikes were placed");
    }

    #[test]
    fn honey_patch_needs_a_real_jungle_majority() {
        let (mut world, _layout) = stone_world(1600, 700, 200);
        let mut structures = StructureMap::new();
        let mut rand = UnifiedRandom::new(5);
        assert!(!honey_patch(
            &mut world,
            &mut structures,
            &mut rand,
            400,
            250
        ));

        for x in 370..430 {
            for y in 220..280 {
                world.set_tile(x, y, Tile::block(tiles::JUNGLE_GRASS));
            }
        }
        assert!(honey_patch(
            &mut world,
            &mut structures,
            &mut rand,
            400,
            250
        ));
        let mut honey_block = 0;
        for x in 370..430 {
            for y in 220..280 {
                if world.tile(x, y).block == tiles::HONEY_BLOCK {
                    honey_block += 1;
                }
            }
        }
        assert!(honey_block > 0, "no honey block was placed");
    }

    #[test]
    fn campsite_places_a_campfire_on_real_surface() {
        let mut world = World::empty(1600, 700, "campsite");
        let mut rand = UnifiedRandom::new(6);
        let mut layout = Layout::plan(1600, 700, &mut rand);
        layout.surface = 100;
        for x in 0..1600 {
            for y in 100..700 {
                world.set_tile(x, y, Tile::block(tiles::STONE));
            }
        }
        let mut structures = StructureMap::new();
        assert!(campsite(
            &mut world,
            &layout,
            &mut structures,
            &mut rand,
            400
        ));
        let mut campfire = false;
        for x in 380..420 {
            for y in 80..110 {
                if world.tile(x, y).block == tiles::CAMPFIRE {
                    campfire = true;
                }
            }
        }
        assert!(campfire, "no campfire was placed");
    }

    #[test]
    fn marble_and_granite_repaint_existing_stone_and_keep_ore() {
        let (mut world, _layout) = stone_world(1600, 700, 200);
        world.set_tile(400, 300, Tile::block(tiles::GOLD));
        let mut structures = StructureMap::new();
        let mut rand = UnifiedRandom::new(7);
        let bounds = paint_ellipse_biome(
            &mut world,
            &mut structures,
            &mut rand,
            400,
            300,
            tiles::MARBLE,
            tiles::walls::MARBLE,
        );
        assert!(bounds.width > 0 && bounds.height > 0);
        assert_eq!(
            world.tile(400, 300).block,
            tiles::GOLD,
            "ore inside the painted area should survive"
        );
        let mut marble = 0;
        for x in (bounds.x)..(bounds.right()) {
            for y in (bounds.y)..(bounds.bottom()) {
                if world.tile(x, y).block == tiles::MARBLE {
                    marble += 1;
                }
            }
        }
        assert!(marble > 0, "no marble was painted");
    }

    #[test]
    fn a_small_world_returns_a_default_report_rather_than_panicking() {
        let mut world = World::empty(300, 200, "tiny");
        let mut rand = UnifiedRandom::new(1);
        let layout = Layout::plan(300, 200, &mut rand);
        let mut structures = StructureMap::new();
        let report = scatter(&mut world, &layout, &mut structures, &mut rand);
        assert_eq!(report, Report::default());
    }

    /// A real 4200x1200 world, generated far enough to have real terrain, caves and evil to site
    /// against — everything [`scatter`] needs.
    fn generated_world(seed: i32) -> (World, Layout) {
        let mut world = World::empty(4200, 1200, "micro-biomes");
        let mut rand = UnifiedRandom::new(seed);
        let layout = Layout::plan(4200, 1200, &mut rand);
        let heights = super::super::terrain::heightmap(&layout, &mut rand);
        super::super::terrain::fill(&mut world, &layout, &heights, &mut rand);
        super::super::structures::caves(&mut world, &layout, &mut rand);
        (world, layout)
    }

    #[test]
    fn a_real_generated_world_places_several_micro_biomes() {
        let (mut world, layout) = generated_world(9);
        let mut structures = StructureMap::new();
        let mut rand = UnifiedRandom::new(10);
        let report = scatter(&mut world, &layout, &mut structures, &mut rand);
        let total = report.thin_ice
            + report.corruption_pits
            + report.spike_pits
            + report.honey_patches
            + report.campsites
            + report.marble
            + report.granite;
        assert!(total > 0, "no micro-biome of any kind was placed");
    }

    /// `cargo test -p terrustia --lib micro_biomes::tests::measure_on_real_worlds --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn measure_on_real_worlds() {
        for seed in [999u64, 4242, 12345] {
            let (mut world, layout) = generated_world(seed as i32);
            let mut structures = StructureMap::new();
            let mut rand = UnifiedRandom::new(seed as i32);
            let report = scatter(&mut world, &layout, &mut structures, &mut rand);
            eprintln!("seed {seed}: {report:?}");
        }
    }
}
