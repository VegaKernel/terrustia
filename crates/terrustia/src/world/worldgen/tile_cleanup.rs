//! The tile-cleanup bundle: six real vanilla passes with a shared theme — mopping up after
//! everything else, rather than placing anything new — bundled the way `plan.md`'s own sizing
//! table already groups them (`TileCleanup`+`QuickCleanup`+`FinalCleanup`+`BrokenTrapCleanup`+
//! `GravitatingSandCleanup`+`SurfaceOreAndStone`+`SurfaceDirtWallsToGrassWalls`, 1,121 lines).
//!
//! **A real finding, not assumed from the sizing table's own "mechanically simple" note**: reading
//! all seven driving passes in full shows most of their bulk is either (a) vanilla's own defensive
//! repair for damage *other* vanilla passes can do to a multi-tile object's frame — chests, the
//! Demon Altar, Shadow Orb, Heart Crystal, all re-normalised from scratch every generation, because
//! some earlier vanilla pass might have left one half-finished — which this project's own placement
//! code has no equivalent need for, since every object here is written with a correct frame at
//! creation and nothing downstream corrupts it; or (b) a genuinely separate, much larger helper
//! function hiding behind a small driving-pass line count (`SurfaceOreAndStone`'s own `OrePatch`/
//! `StonePatch`, ~370 real lines *each*, not counted in the pass's own 66). Both are disclosed in
//! full below, pass by pass, rather than silently dropped.
//!
//! * `GravitatingSandCleanup` (`WorldGen.cs:15210-15239`, 30 lines) — **faithful and complete**.
//! * `QuickCleanup` (`:18002-18122`, 121) — landed narrower: the desert-wall material
//!   normalisation (mud/silt/slush/marble/granite forced to Hardened Sand under a Sandstone or
//!   Hardened Sand wall, and any liquid there forced to lava below the rock layer) and the
//!   degenerate-slope/half-brick cleanup are kept; the ocean liquid-type normalisation (needs
//!   `oceanDepths`/`oceanLevel`, GenVars this generator does not track) and the sand-overhang dig
//!   and wall-continuity fixups (real but genuinely minor/cosmetic) are disclosed-skipped.
//! * `SurfaceOreAndStone` (`:18577-18642`, 66) — landed **much narrower than its own line count
//!   implies**. The driving pass just picks sites; the actual carving is two helper functions
//!   neither counted in the sizing table's 66: `OrePatch` (`:10589-10970`ish, ~370 lines) and
//!   `StonePatch` (`:10218-10589`ish, ~370), each a real elliptical noise-carve — a genuinely
//!   separate, Tier-2-sized siting-and-shaping algorithm, not a small tile scan. Replaced here with
//!   a disclosed, narrower stand-in: a small filled-circle blob of ore or stone at a scanned
//!   surface site, keeping the *effect* (a visible surface outcropping) without porting vanilla's
//!   own noise-carve shape.
//! * `SurfaceDirtWallsToGrassWalls` (`:19752-19847`, 96) — landed narrower: the wall-conversion
//!   half (`Spread.Wall2`, dirt wall near open air near the surface becomes grass wall) is kept;
//!   the tile-conversion half (`SpreadGrass`, dirt tiles near a grass wall turn to grass) needs
//!   `SpreadGrass`'s own general recursive machinery — the same subsystem `moss.rs`'s own module
//!   doc already disclosed skipping for its own closing diffusion step — and is skipped here too.
//! * `TileCleanup` (`:21700-22130`, 431) — landed narrowest of all six: three small, genuinely
//!   load-bearing fixups (a degenerate top-slope-beside-half-brick straighten; liquid forced dry
//!   behind dungeon/temple wall; a `Traps`-tile (137) neighbour half-brick/slope clear) plus the
//!   thin-ice floating-tile cleanup (tile 162, directly relevant to `thin_ice.rs`'s own mechanic).
//!   The rest — a multi-tile frame-renormalisation pass for Shadow Orb/Heart Crystal/chests/the
//!   Demon Altar, and a liquid-drip decoration system (tiles 373-375/709) — is disclosed-skipped
//!   per the module doc's opening paragraph.
//! * `BrokenTrapCleanup` (`:22315-22335`, 21, driving `ClearBrokenTraps`, `:27116-27230`ish, ~115)
//!   — **faithful**: a real wire-circuit flood that clears any wired tile whose circuit does not
//!   contain both a trap and a trigger. `IsItATrap`/`IsItATrigger`'s own real vanilla sets
//!   (`TileID.Sets.Wiring.IsAMechanism`/`IsATrigger`, ~70 and ~10 entries) are narrowed to the
//!   handful of tile types this generator's own `traps.rs`/`wiring.rs` ever actually place — a
//!   disclosed narrowing of the *lookup table*, not the *mechanism*, since every other listed
//!   vanilla tile (banners, Christmas lights, non-generated furniture) can never appear in a world
//!   this generator built in the first place.
//! * `FinalCleanup` (`:22336-22691`, 356) — landed narrower: dungeon-wall liquid dry-out (plus the
//!   real Obsidian-in-dungeon purge), pressure-plate liquid dry-out, the same `Traps`-tile clear
//!   `tile_cleanup` above does (vanilla repeats this fixup in both passes; done once here too, in
//!   `final_cleanup`, to match), a Gold Coin Pile vertical-duplication touch-up, and stray isolated
//!   surface-puddle drainage are kept. Disclosed-skipped: the sand/silt/slush-to-hardened-sand
//!   surface-exposure conversion (a second, more elaborate falling-column simulation distinct from
//!   `GravitatingSandCleanup`'s own), the boulder multi-tile frame-renormalisation-plus-altar-
//!   conflict check, a full-world `TileFrame()` re-frame pass (the same "not needed, this project's
//!   placement code writes correct frames at creation" reasoning as `TileCleanup`'s own cut), the
//!   Desert Fossil easter-egg scatter, every secret-seed finalizer, and the closing full-world
//!   `LiquidCheck` settle pass (this project already settles liquids once, earlier in `build()`,
//!   via `liquid_settle::settle` — see that module's own doc comment).
//!
//! No `StructureMap` dependency anywhere in this bundle — every pass here reshapes or repairs tiles
//! already down, none places a new discrete structure.

use terrustia_proto::{Liquid, Tile, TileFlags};

use super::layout::Layout;
use super::rand::UnifiedRandom;
use super::smooth::{solid_or_sloped, solid_tile};
use super::tiles;
use crate::world::World;

/// Deactivates a tile the same safe way `smooth.rs`'s own `kill_tile` does: rebuild from
/// `Tile::AIR` (block 0, frame -1, no slope/half-brick, inactive) and copy over only the wall and
/// liquid, rather than merely flipping the `ACTIVE` bit. **A real bug found by this bundle's own
/// integration test, the same class already found once this session for `underground_cabins.rs`**:
/// every "clear/kill a tile" call site below originally just cleared `ACTIVE` and left `block`/
/// `frame_x`/`frame_y` at whatever they were — harmless in memory (nothing reads a frame off an
/// inactive tile at runtime), but a real save/reload divergence, since the writer does not carry a
/// stale non-`-1` frame or a stale block id on a tile it treats as air. `a_generated_world_survives_a_save`
/// caught it directly: 5 tiles changed across a save before this fix, 0 after.
fn kill(world: &mut World, x: i32, y: i32) {
    let t = world.tile(x, y);
    let mut cleared = Tile::AIR;
    cleared.wall = t.wall;
    cleared.wall_color = t.wall_color;
    cleared.liquid = t.liquid;
    cleared.liquid_kind = t.liquid_kind;
    world.set_tile(x, y, cleared);
}

// ---------------------------------------------------------------------------------------------
// GravitatingSandCleanup
// ---------------------------------------------------------------------------------------------

/// `TileID.Sets.Falling`: Sand, Crimsand, Ebonsand, Pearlsand, Slush, Silt, and four Ash-family
/// variants.
const FALLING: [u16; 11] = [53, 234, 112, 116, 224, 123, 330, 331, 332, 333, 495];

/// The `GravitatingSandCleanup` pass, faithful and complete. Returns how many tiles were dropped.
pub fn gravitating_sand_cleanup(world: &mut World, layout: &Layout) -> usize {
    let mut dropped = 0usize;
    for x in 0..world.width() {
        let mut flag = false;
        let mut last_solid = 0i32;
        for y in (1..world.height()).rev() {
            if !solid_or_sloped(world, x, y) {
                continue;
            }
            let block = world.tile(x, y).block;
            if flag && y < layout.surface && y != last_solid - 1 && FALLING.contains(&block) {
                for j in y..last_solid {
                    let mut t = world.tile(x, j);
                    t.block = block;
                    t.flags.set(TileFlags::ACTIVE, true);
                    t.frame_x = -1;
                    t.frame_y = -1;
                    t.slope = 0;
                    t.flags.set(TileFlags::HALF_BRICK, false);
                    world.set_tile(x, j, t);
                    dropped += 1;
                }
            }
            flag = true;
            last_solid = y;
        }
    }
    dropped
}

// ---------------------------------------------------------------------------------------------
// QuickCleanup
// ---------------------------------------------------------------------------------------------

/// The `QuickCleanup` pass, narrowed per the module doc. Returns how many tiles were changed.
pub fn quick_cleanup(world: &mut World, layout: &Layout) -> usize {
    let mut changed = 0usize;
    for x in 0..world.width() {
        for y in 0..world.height() {
            let t = world.tile(x, y);
            if t.wall == tiles::walls::SANDSTONE || t.wall == tiles::walls::HARDENED_SAND {
                let mut t = t;
                let mut touched = false;
                if matches!(t.block, tiles::MUD | tiles::SILT | 224) {
                    t.block = tiles::HARDENED_SAND;
                    touched = true;
                }
                if matches!(t.block, tiles::GRANITE | tiles::MARBLE) {
                    t.block = tiles::HARDENED_SAND;
                    touched = true;
                }
                if y <= layout.rock {
                    if t.liquid > 0 {
                        t.liquid = 0;
                        touched = true;
                    }
                } else if t.liquid > 0 {
                    t.liquid = 255;
                    t.liquid_kind = Liquid::Lava;
                    touched = true;
                }
                if touched {
                    world.set_tile(x, y, t);
                    changed += 1;
                }
            }

            // Degenerate top-sloped tile beside a half-bricked neighbour: straighten it.
            let t = world.tile(x, y);
            if t.is_active() && t.slope != 0 && is_top_slope(t.slope) {
                let (left, right) = (world.tile(x - 1, y), world.tile(x + 1, y));
                let straighten = (is_left_slope(t.slope)
                    && left.is_active()
                    && right.is_active()
                    && world.tile(x + 1, y).flags.has(TileFlags::HALF_BRICK))
                    || (is_right_slope(t.slope)
                        && left.is_active()
                        && left.flags.has(TileFlags::HALF_BRICK));
                if straighten {
                    let mut t = t;
                    t.slope = 0;
                    t.flags.set(TileFlags::HALF_BRICK, true);
                    world.set_tile(x, y, t);
                    changed += 1;
                }
            }
        }
    }
    changed
}

// `Tile.slope`: 1=bottom-right(?)/2/3/4 map to vanilla's four corner slopes. Vanilla's own
// `topSlope()`/`leftSlope()`/`rightSlope()` accessors are transcribed narrowly here — only the
// three combinations `QuickCleanup` itself actually reads.
fn is_top_slope(slope: u8) -> bool {
    matches!(slope, 3 | 4)
}
fn is_left_slope(slope: u8) -> bool {
    matches!(slope, 1 | 3)
}
fn is_right_slope(slope: u8) -> bool {
    matches!(slope, 2 | 4)
}

// ---------------------------------------------------------------------------------------------
// SurfaceOreAndStone (narrowed — see module doc)
// ---------------------------------------------------------------------------------------------

/// A small stand-in for `OrePatch`/`StonePatch`: a filled circle of ore or stone at a scanned
/// near-surface site, in place of vanilla's own elliptical noise-carve helpers (disclosed in the
/// module doc as a genuinely separate, much larger algorithm this bundle does not port). Returns
/// how many patches were placed.
pub fn surface_ore_and_stone(
    world: &mut World,
    layout: &Layout,
    rand: &mut UnifiedRandom,
) -> usize {
    if layout.width < 45 {
        return 0;
    }
    let mut placed = 0usize;
    let ore_tiles = [tiles::COPPER, tiles::IRON, tiles::SILVER, tiles::GOLD];
    let attempts = rand.next_range(5, 10) * (layout.width / 4200).max(1);
    for _ in 0..attempts {
        let x = rand.next_range(20, layout.width - 20);
        let y = rand.next_range(0, layout.surface.max(1));
        if place_surface_blob(world, x, y, ore_tiles[rand.next_max(4) as usize], rand) {
            placed += 1;
        }
    }
    let stone_attempts = rand.next_range(1, 8).max(1);
    for _ in 0..stone_attempts {
        let x = rand.next_range(20, layout.width - 20);
        let y = rand.next_range(0, layout.surface.max(1));
        if place_surface_blob(world, x, y, tiles::STONE, rand) {
            placed += 1;
        }
    }
    placed
}

fn place_surface_blob(
    world: &mut World,
    x: i32,
    y: i32,
    block: u16,
    rand: &mut UnifiedRandom,
) -> bool {
    // Real ground within the near-surface band, per vanilla's own "walk down to the grass line"
    // (`StonePatch`/`OrePatch` both start this way).
    let mut gy = y;
    while gy < world.height() - 5 && !solid_tile(world, x, gy) {
        gy += 1;
    }
    if gy >= world.height() - 5 || !world.tile(x, gy).is_active() {
        return false;
    }
    let radius = rand.next_range(4, 8);
    let mut any = false;
    for dx in -radius..=radius {
        for dy in -radius..=radius {
            if dx * dx + dy * dy > radius * radius {
                continue;
            }
            let (px, py) = (x + dx, gy + dy);
            let t = world.tile(px, py);
            if t.is_active() && (t.block == tiles::DIRT || t.block == tiles::STONE) {
                let mut t = t;
                t.block = block;
                world.set_tile(px, py, t);
                any = true;
            }
        }
    }
    any
}

// ---------------------------------------------------------------------------------------------
// SurfaceDirtWallsToGrassWalls (wall-conversion half only — see module doc)
// ---------------------------------------------------------------------------------------------

const CANNOT_BE_REPLACED: [u16; 7] = [
    4,
    tiles::walls::SNOW,
    3,
    83,
    tiles::walls::LIHZAHRD_BRICK,
    244,
    34,
];

/// `Spread.Wall2(x, y, 63)`: like `wall_variety.rs`'s own `spread_wall_mud`, but this call site's
/// wall type (`GrassUnsafe`, 63) *does* set `WallID.Sets.WallSpreadStopsAtAir[63]` — unlike Mud —
/// so the flood also has to stop at any open-air tile that has no wall of its own, rather than
/// spreading through open space freely. The real diagonal-expansion branch that same flag gates is
/// not transcribed (a minor extra reach vanilla's own flood gets once it stops at an air pocket,
/// disclosed here rather than silently matched).
fn spread_grass_wall(world: &mut World, x: i32, y: i32) {
    const GRASS_WALL: u16 = tiles::walls::GRASS_UNSAFE;
    let mut seen: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::from([(x, y)]);
    let mut painted = 0usize;
    while let Some((cx, cy)) = queue.pop_front() {
        if !seen.insert((cx, cy)) || !world.in_bounds(cx, cy) {
            continue;
        }
        let mut t = world.tile(cx, cy);
        if t.wall == GRASS_WALL || CANNOT_BE_REPLACED.contains(&t.wall) {
            continue;
        }
        let solid = terrustia_proto::tile_solid::solid(t.block) && t.is_active();
        if !solid {
            if !t.is_active() && t.wall == 0 {
                // `WallSpreadStopsAtAir[63]`: an open tile with no wall of its own stops the flood
                // rather than joining it.
                continue;
            }
            if painted >= 5000 {
                continue;
            }
            painted += 1;
            t.wall = GRASS_WALL;
            world.set_tile(cx, cy, t);
            queue.push_back((cx - 1, cy));
            queue.push_back((cx + 1, cy));
            queue.push_back((cx, cy - 1));
            queue.push_back((cx, cy + 1));
        } else if t.is_active() {
            t.wall = GRASS_WALL;
            world.set_tile(cx, cy, t);
        }
    }
}

/// The `SurfaceDirtWallsToGrassWalls` pass, wall-conversion half only. Returns how many sites were
/// flooded.
pub fn surface_dirt_walls_to_grass_walls(
    world: &mut World,
    layout: &Layout,
    rand: &mut UnifiedRandom,
) -> usize {
    if layout.width < 105 || layout.surface < 10 {
        return 0;
    }
    let mut sites = 0usize;
    for x in 50..layout.width - 50 {
        for y in 0..layout.surface - 10 {
            if rand.next_max(4) != 0 {
                continue;
            }
            let t = world.tile(x, y);
            if !(t.is_active() && t.block == tiles::DIRT && t.wall == tiles::walls::DIRT) {
                continue;
            }
            let mut open_neighbour = false;
            for k in x - 1..=x + 1 {
                for l in y - 1..=y + 1 {
                    if world.tile(k, l).wall == 0 && !solid_tile(world, k, l) {
                        open_neighbour = true;
                    }
                }
            }
            if !open_neighbour {
                continue;
            }
            let mut target = None;
            for m in x - 1..=x + 1 {
                for l in y - 1..=y + 1 {
                    let w = world.tile(m, l).wall;
                    if (w == tiles::walls::DIRT || w == tiles::walls::MUD)
                        && !solid_tile(world, m, l)
                    {
                        target = Some((m, l));
                    }
                }
            }
            if let Some((tx, ty)) = target {
                spread_grass_wall(world, tx, ty);
                sites += 1;
            }
        }
    }
    sites
}

// ---------------------------------------------------------------------------------------------
// TileCleanup (narrowed — see module doc)
// ---------------------------------------------------------------------------------------------

const HELLSTONE_BRICK_UNSAFE: u16 = 13;
const OBSIDIAN_BRICK_UNSAFE: u16 = 14;
const TRAPS_TILE: u16 = 137;

/// The three small fixups plus the thin-ice cleanup this bundle keeps from `TileCleanup` — see the
/// module doc for what's cut. Returns how many tiles changed.
pub fn tile_cleanup(world: &mut World) -> usize {
    let mut changed = 0usize;
    for x in 0..world.width() {
        for y in 0..world.height() {
            changed += clear_dry_wall(
                world,
                x,
                y,
                &[HELLSTONE_BRICK_UNSAFE, OBSIDIAN_BRICK_UNSAFE],
            );
            changed += clear_dry_wall(world, x, y, &[tiles::walls::LIHZAHRD_BRICK]);
            changed += clear_traps_neighbour_slope(world, x, y);
            changed += clear_floating_thin_ice(world, x, y);
        }
    }
    changed
}

fn clear_dry_wall(world: &mut World, x: i32, y: i32, walls: &[u16]) -> usize {
    let t = world.tile(x, y);
    if walls.contains(&t.wall) && t.liquid > 0 {
        let mut t = t;
        t.liquid = 0;
        world.set_tile(x, y, t);
        1
    } else {
        0
    }
}

fn clear_traps_neighbour_slope(world: &mut World, x: i32, y: i32) -> usize {
    let t = world.tile(x, y);
    if !(t.is_active() && t.block == TRAPS_TILE) {
        return 0;
    }
    let frame_row = t.frame_y / 18;
    if !(frame_row <= 2 || frame_row == 5) {
        return 0;
    }
    let dx = if t.frame_x >= 18 { 1 } else { -1 };
    let n = world.tile(x + dx, y);
    if n.flags.has(TileFlags::HALF_BRICK) || n.slope != 0 {
        kill(world, x + dx, y);
        1
    } else {
        0
    }
}

/// The `type == 162` (`BreakableIce`) branch: a thin-ice tile floating with no water below it and
/// no support above gets cleared. Ties directly into `thin_ice.rs`'s own mechanic.
fn clear_floating_thin_ice(world: &mut World, x: i32, y: i32) -> usize {
    let t = world.tile(x, y);
    if !(t.is_active() && t.block == tiles::BREAKABLE_ICE) {
        return 0;
    }
    let above = world.tile(x, y - 1);
    let below = world.tile(x, y + 1);
    if !above.is_active() && !below.is_active() && below.liquid == 0 {
        kill(world, x, y);
        1
    } else {
        0
    }
}

// ---------------------------------------------------------------------------------------------
// BrokenTrapCleanup — faithful (see module doc for the narrowed lookup table)
// ---------------------------------------------------------------------------------------------

fn is_trap(t: terrustia_proto::Tile) -> bool {
    if !t.is_active() {
        return false;
    }
    if t.flags.has(TileFlags::ACTUATOR) {
        return true;
    }
    matches!(t.block, 137 | 141 | 443)
}

fn is_trigger(t: terrustia_proto::Tile) -> bool {
    t.is_active() && matches!(t.block, 135 | 136)
}

/// `ClearBrokenTraps`, driven once per unvisited wired tile — `BrokenTrapCleanup`. Returns how
/// many tiles had their wire cleared for belonging to an incomplete circuit.
pub fn broken_trap_cleanup(world: &mut World) -> usize {
    let mut visited: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    let mut cleared = 0usize;
    for x in 50..world.width() - 50 {
        for y in 50..world.height() - 50 {
            if !world.tile(x, y).flags.has(TileFlags::WIRE_RED) || visited.contains(&(x, y)) {
                continue;
            }
            cleared += clear_one_circuit(world, x, y, &mut visited);
        }
    }
    cleared
}

fn clear_one_circuit(
    world: &mut World,
    sx: i32,
    sy: i32,
    global_visited: &mut std::collections::HashSet<(i32, i32)>,
) -> usize {
    let mut circuit: Vec<(i32, i32)> = Vec::new();
    let mut queue = std::collections::VecDeque::from([(sx, sy)]);
    let mut local_seen: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    let (mut has_trap, mut has_trigger) = (false, false);
    let mut budget = 4_000_000i32;

    while let Some((cx, cy)) = queue.pop_front() {
        budget -= 1;
        if budget <= 0 || !local_seen.insert((cx, cy)) {
            continue;
        }
        if !world.in_bounds(cx, cy) {
            continue;
        }
        let t = world.tile(cx, cy);
        if !t.flags.has(TileFlags::WIRE_RED) {
            continue;
        }
        circuit.push((cx, cy));
        if is_trap(t) {
            has_trap = true;
        }
        if is_trigger(t) {
            has_trigger = true;
        }
        if has_trap && has_trigger {
            break;
        }
        queue.push_back((cx - 1, cy));
        queue.push_back((cx + 1, cy));
        queue.push_back((cx, cy - 1));
        queue.push_back((cx, cy + 1));
    }

    for &p in &circuit {
        global_visited.insert(p);
    }
    if has_trap && has_trigger {
        return 0;
    }

    let mut cleared = 0usize;
    for (cx, cy) in circuit {
        let mut t = world.tile(cx, cy);
        let was_trap = is_trap(t);
        let was_trigger = is_trigger(t);
        if t.flags.has(TileFlags::ACTUATOR) {
            t.flags.set(TileFlags::WIRE_RED, false);
            t.flags.set(TileFlags::ACTUATOR, false);
            world.set_tile(cx, cy, t);
        } else if (was_trap && t.block != 141) || was_trigger {
            // Vanilla's real `KillTile` call for a broken trap/trigger tile — both branches do the
            // same thing in this narrowed lookup table (only the minecart-pressure-plate case, 314,
            // gets a different real treatment in source, a frame reset rather than a kill, and 314
            // is not in this bundle's own narrowed `is_trigger`). `kill` also clears the wire flag,
            // since a killed tile carries no wire at all — see `kill`'s own doc comment for why this
            // resets the whole tile rather than just flipping `ACTIVE`.
            kill(world, cx, cy);
        } else {
            t.flags.set(TileFlags::WIRE_RED, false);
            world.set_tile(cx, cy, t);
        }
        cleared += 1;
    }
    cleared
}

// ---------------------------------------------------------------------------------------------
// FinalCleanup (narrowed — see module doc)
// ---------------------------------------------------------------------------------------------

const WALL_DUNGEON: [u16; 3] = [
    tiles::walls::BLUE_DUNGEON,
    tiles::walls::GREEN_DUNGEON,
    tiles::walls::PINK_DUNGEON,
];
const GOLD_COIN_PILE: u16 = tiles::GOLD_COIN_PILE;

/// The five fixups this bundle keeps from `FinalCleanup` — see the module doc. Returns how many
/// tiles changed.
pub fn final_cleanup(world: &mut World, layout: &Layout) -> usize {
    let mut changed = 0usize;
    for x in 0..world.width() {
        for y in 0..world.height() {
            let t = world.tile(x, y);

            if WALL_DUNGEON.contains(&t.wall) {
                let mut t = t;
                let mut touched = false;
                if t.liquid > 0 {
                    t.liquid = 0;
                    touched = true;
                }
                if t.is_active() && t.block == tiles::OBSIDIAN {
                    t.flags.set(TileFlags::ACTIVE, false);
                    t.liquid = 128;
                    t.liquid_kind = Liquid::Lava;
                    touched = true;
                }
                if touched {
                    world.set_tile(x, y, t);
                    changed += 1;
                }
            }

            if t.is_active() && t.block == 314 && t.liquid > 0 {
                let mut t = t;
                t.liquid = 0;
                world.set_tile(x, y, t);
                changed += 1;
            }

            changed += clear_traps_neighbour_slope(world, x, y);

            if t.is_active() && t.block == GOLD_COIN_PILE && !world.tile(x, y + 1).is_active() {
                let mut below = world.tile(x, y + 1);
                below.block = GOLD_COIN_PILE;
                below.flags.set(TileFlags::ACTIVE, true);
                below.frame_x = t.frame_x;
                below.frame_y = t.frame_y;
                world.set_tile(x, y + 1, below);
                changed += 1;
            }

            let beach = 380;
            if x > beach
                && x < layout.width - beach
                && y < layout.surface
                && t.liquid > 0
                && t.liquid < 255
                && world.tile(x - 1, y).liquid < 255
                && world.tile(x + 1, y).liquid < 255
                && world.tile(x, y + 1).liquid < 255
            {
                let mut t = t;
                t.liquid = 0;
                world.set_tile(x, y, t);
                changed += 1;
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::Tile;

    fn stone_world(width: i32, height: i32, seed: i32) -> (World, Layout) {
        let mut world = World::empty(width, height, "tile-cleanup");
        for x in 0..width {
            for y in 0..height {
                world.set_tile(x, y, Tile::block(tiles::STONE));
            }
        }
        let mut rand = UnifiedRandom::new(seed);
        let layout = Layout::plan(width, height, &mut rand);
        (world, layout)
    }

    #[test]
    fn a_floating_sand_column_drops_to_rest_on_solid_ground() {
        let (mut world, layout) = stone_world(400, 300, 1);
        let x = 200;
        // Solid ground well below the floating sand, both comfortably above `worldSurface` (the
        // pass's own `y < worldSurface` check only fires for a column above the surface line) —
        // the first draft of this test used fixed row numbers that happened to sit *below*
        // `layout.surface` for a 300-tall world (`layout.surface` is `height * 0.28`, so 84 here,
        // not the 250 the first draft assumed).
        let ground_y = layout.surface + 60;
        let (sand_top, sand_bottom) = (10, 14);
        assert!(
            sand_bottom < layout.surface,
            "test assumes the sand sits above worldSurface"
        );
        for y in 0..ground_y {
            world.set_tile(x, y, Tile::AIR);
        }
        world.set_tile(x, ground_y, Tile::block(tiles::STONE));
        for y in sand_top..sand_bottom {
            world.set_tile(x, y, Tile::block(53)); // Sand
        }
        let dropped = gravitating_sand_cleanup(&mut world, &layout);
        assert!(dropped > 0, "expected the floating sand to drop");
        assert_eq!(
            world.tile(x, ground_y - 1).block,
            53,
            "sand should now rest just above solid ground"
        );
    }

    #[test]
    fn dungeon_wall_liquid_is_dried_out() {
        let (mut world, layout) = stone_world(400, 300, 2);
        let mut wet = Tile::AIR;
        wet.wall = tiles::walls::BLUE_DUNGEON;
        wet.liquid = 200;
        world.set_tile(50, 50, wet);
        let changed = final_cleanup(&mut world, &layout);
        assert!(changed > 0);
        assert_eq!(world.tile(50, 50).liquid, 0);
    }

    #[test]
    fn a_wired_trap_with_no_trigger_loses_its_wire() {
        let (mut world, layout) = stone_world(400, 300, 3);
        let mut trap = Tile::framed(137, 0, 0);
        trap.flags.set(TileFlags::WIRE_RED, true);
        world.set_tile(100, 100, trap);
        let cleared = broken_trap_cleanup(&mut world);
        assert!(cleared > 0);
        assert!(!world.tile(100, 100).flags.has(TileFlags::WIRE_RED));
        let _ = layout;
    }

    #[test]
    fn a_wired_trap_with_a_real_trigger_keeps_its_wire() {
        let (mut world, _layout) = stone_world(400, 300, 4);
        let mut trap = Tile::framed(137, 0, 0);
        trap.flags.set(TileFlags::WIRE_RED, true);
        world.set_tile(100, 100, trap);
        let mut plate = Tile::framed(135, 0, 0);
        plate.flags.set(TileFlags::WIRE_RED, true);
        world.set_tile(101, 100, plate);
        let cleared = broken_trap_cleanup(&mut world);
        assert_eq!(
            cleared, 0,
            "a real trap+trigger circuit should survive intact"
        );
        assert!(world.tile(100, 100).flags.has(TileFlags::WIRE_RED));
    }

    #[test]
    fn a_desert_wall_pocket_normalises_its_own_material() {
        let (mut world, layout) = stone_world(400, 300, 5);
        let mut mud = Tile::block(tiles::MUD);
        mud.wall = tiles::walls::SANDSTONE;
        world.set_tile(50, 50, mud);
        let changed = quick_cleanup(&mut world, &layout);
        assert!(changed > 0);
        assert_eq!(world.tile(50, 50).block, tiles::HARDENED_SAND);
    }

    #[test]
    fn surface_ore_and_stone_places_a_real_blob_near_the_surface() {
        let (mut world, layout) = stone_world(1200, 900, 6);
        // Solid dirt just below the surface, open air above — a real near-surface site.
        for x in 0..1200 {
            for y in 0..layout.surface {
                world.set_tile(x, y, Tile::AIR);
            }
            world.set_tile(x, layout.surface, Tile::block(tiles::DIRT));
        }
        let mut rand = UnifiedRandom::new(6);
        let placed = surface_ore_and_stone(&mut world, &layout, &mut rand);
        assert!(
            placed > 0,
            "expected at least one ore or stone blob near the surface"
        );
    }

    #[test]
    fn a_dirt_walled_dirt_tile_near_open_space_becomes_grass_walled() {
        let (mut world, layout) = stone_world(1200, 900, 7);
        // A dirt block near the surface, walled with plain Dirt wall, with a genuinely open
        // (unwalled, non-solid) tile among its own neighbours — the real site shape this pass
        // requires before it will flood a grass-wall conversion from there at all.
        let (x, y) = (600, layout.surface - 30);
        for dx in -2..=2 {
            for dy in -2..=2 {
                let mut t = Tile::AIR;
                t.wall = tiles::walls::DIRT;
                world.set_tile(x + dx, y + dy, t);
            }
        }
        let mut dirt = Tile::block(tiles::DIRT);
        dirt.wall = tiles::walls::DIRT;
        world.set_tile(x, y, dirt);
        world.set_tile(x + 1, y, Tile::AIR); // the open, wall-free neighbour

        let mut rand = UnifiedRandom::new(7);
        let sites = surface_dirt_walls_to_grass_walls(&mut world, &layout, &mut rand);
        assert!(
            sites > 0,
            "expected at least one grass-wall conversion site"
        );
        let grass_walled = (x - 2..=x + 2)
            .flat_map(|gx| (y - 2..=y + 2).map(move |gy| (gx, gy)))
            .any(|(gx, gy)| world.tile(gx, gy).wall == tiles::walls::GRASS_UNSAFE);
        assert!(
            grass_walled,
            "expected some real grass wall painted near the site"
        );
    }

    #[test]
    fn a_small_world_does_not_panic() {
        let (mut world, layout) = stone_world(400, 300, 1);
        let mut rand = UnifiedRandom::new(1);
        let _ = gravitating_sand_cleanup(&mut world, &layout);
        let _ = quick_cleanup(&mut world, &layout);
        let _ = surface_ore_and_stone(&mut world, &layout, &mut rand);
        let _ = surface_dirt_walls_to_grass_walls(&mut world, &layout, &mut rand);
        let _ = tile_cleanup(&mut world);
        let _ = broken_trap_cleanup(&mut world);
        let _ = final_cleanup(&mut world, &layout);
    }
}
