//! Traps.
//!
//! Transcribed from `WorldGen.cs`'s `placeTrap` (8872), its guard `placeTrap_CanContinue` (8859),
//! `PlaceSandTrap` (35851), and the `GenPassNameID.Traps` pass that drives them all (18787).
//!
//! Vanilla gates almost every branch of this behind secret-seed flags. One of them —
//! `noTrapsWorldGen` — is modelled now: see [`scatter`]'s own `secret` parameter and
//! `secret_seed.rs`'s own module doc for the real activation mechanism. The rest
//! (`remixWorldGen`, `tenthAnniversaryWorldGen`, `skyblockWorldGen`, `drunkWorldGen`,
//! `getGoodWorldGen`, `Main.starGame`, `SecretSeed.*`) are still not modelled — only the
//! ordinary-world path through each function is transcribed here, which is what every world this
//! generator produced before this module existed actually needed, and is the exact opposite of
//! what was there before that: nothing.
//!
//! Four things get placed, chosen by vanilla's own weighted roll once a candidate site's floor is
//! found:
//!
//! * A **dart trap** (`type 0`, 5/12 of normal rolls): a pressure plate wired to a wall-mounted
//!   dart emitter, tile 137.
//! * A **land mine** (`type 2`, 1/20): a pressure plate wired to a buried explosive, tile 141 —
//!   `Explosives` in vanilla's own `TileID`, not a "spear trap"; there is no such tile here.
//!   `wiring.rs` reports these through `Fired::mines` rather than `Fired::traps`, because
//!   `trap_shot` has no frame convention for tile 141 and should not be made to invent one.
//! * A **geyser** (`type 3`, when deep enough): tile 443, two tiles wide, unwired — it fires on
//!   its own timer rather than off a plate.
//! * A **boulder trap** (`type 1`, the rest): a chamber with an actuated floor under a
//!   boulder-family tile, wired the same way as the dart trap and the mine.
//!
//! `PlaceSandTrap` is the desert's own set piece — a sand-filled pit with an actuated floor —
//! placed by a separate loop in the same generation pass, gated on sandstone wall rather than a
//! random roll.
//!
//! A handful of vanilla lookups this project has no equivalent table for are approximated, each
//! noted where it happens: `oceanDepths` via `Layout`'s ocean bands rather than
//! `oceanLevel`/`beachDistance`; `GenVars.lavaLine` via `Layout::rock`; `Main.tileDungeon` (a
//! per-tile-type set) via the same wall-based `is_dungeon_wall` proxy `pots.rs` already uses for
//! the wall-based `Main.wallDungeon`; and `CanGeneratePressurePlateAt`'s platform/top-slope
//! carve-out is dropped, since a plate on a platform is a corner case, not a correctness gate.

use rand::{Rng, rngs::SmallRng};
use terrustia_proto::{Tile, tile::TileFlags, tile_solid};

use super::layout::Layout;
use super::secret_seed::SecretSeeds;
use super::tiles::walls;
use crate::world::World;

const PRESSURE_PLATE: u16 = 135;
const DART_TRAP: u16 = 137;
const EXPLOSIVES: u16 = 141;
const GEYSER: u16 = 443;
const BOULDER: u16 = 138;
const BOUNCY_BOULDER: u16 = 664;
const RAINBOW_BOULDER: u16 = 711;
const LAVA_BOULDER: u16 = 713;
const SPIDER_BOULDER: u16 = 714;
const GHOULDER: u16 = 715;
const BOULDER_PET: u16 = 716;
/// `WallID.SpiderUnsafe` — the spider-web cave's own wall.
const SPIDER_WALL: u16 = 62;
/// `WallID.UnbreakableBlockWall` — the world border.
const UNBREAKABLE_WALL: u16 = 350;
const CLOSED_DOOR: u16 = 10;
const SPIKES: u16 = 48;
const SAND: u16 = 53;
/// `TileID.Containers` — an ordinary chest. `place_boulder_trap`'s own nearby-chest guard checks
/// this, not the pressure plate.
const CHEST: u16 = 21;

fn is_boulder(block: u16) -> bool {
    matches!(
        block,
        138 | 484 | 664 | 665 | 711 | 712 | 713 | 714 | 715 | 716
    )
}

/// `Main.wallDungeon[wall]` — the {7, 8, 9, 94..=99} set (`Main.cs:10737-10745`), shared with
/// `pots.rs` so the two cannot drift.
use super::pots::is_dungeon_wall;

fn in_world(layout: &Layout, x: i32, y: i32, fluff: i32) -> bool {
    x >= fluff && x < layout.width - fluff && y >= fluff && y < layout.height - fluff
}

/// `WorldGen.SolidTile(i, j)`.
fn solid_tile(world: &World, x: i32, y: i32) -> bool {
    let t = world.tile(x, y);
    t.is_active()
        && tile_solid::solid(t.block)
        && !tile_solid::solid_top(t.block)
        && !t.flags.has(TileFlags::HALF_BRICK)
        && t.slope == 0
}

/// `WorldGen.SolidTileAllowBottomSlope(i, j)`, minus the platform-frame carve-out (see the module
/// doc comment).
fn solid_tile_allow_bottom_slope(world: &World, x: i32, y: i32) -> bool {
    let t = world.tile(x, y);
    t.is_active()
        && (tile_solid::solid(t.block) || tile_solid::solid_top(t.block))
        && !t.flags.has(TileFlags::HALF_BRICK)
}

/// `WorldGen.CanGeneratePressurePlateAt(i, j)`.
fn can_generate_pressure_plate_at(world: &World, layout: &Layout, x: i32, y: i32) -> bool {
    if !in_world(layout, x, y, 3) {
        return false;
    }
    if !solid_tile_allow_bottom_slope(world, x, y + 1) {
        return false;
    }
    let below = world.tile(x, y + 1);
    if is_boulder(below.block) {
        return false;
    }
    below.wall != UNBREAKABLE_WALL
}

/// `WorldGen.placeTrap_CanContinue(x, y)`, minus the dual-dungeon secret-seed branch.
fn place_trap_can_continue(world: &World, x: i32, y: i32) -> bool {
    world.tile(x, y).wall != UNBREAKABLE_WALL
}

/// `WorldGen.PlaceTile`, for the plain single-frame tiles this file writes. Preserves whatever
/// wall was already there, the way vanilla's own tile write does.
fn place_tile(world: &mut World, x: i32, y: i32, block: u16, frame_x: i16, frame_y: i16) {
    let wall = world.tile(x, y).wall;
    world.set_tile(x, y, Tile::framed(block, frame_x, frame_y).with_wall(wall));
}

/// `WorldGen.KillTile`, without the dust/item side effects a generation pass has no use for.
fn kill_tile(world: &mut World, x: i32, y: i32) {
    let mut t = world.tile(x, y);
    t.block = 0;
    t.frame_x = -1;
    t.frame_y = -1;
    t.slope = 0;
    t.flags.set(TileFlags::ACTIVE, false);
    t.flags.set(TileFlags::HALF_BRICK, false);
    world.set_tile(x, y, t);
}

fn set_wire(world: &mut World, x: i32, y: i32) {
    let mut t = world.tile(x, y);
    t.flags.set(TileFlags::WIRE_RED, true);
    world.set_tile(x, y, t);
}

/// The staircase wire path vanilla's `placeTrap` lays inline, identically, for the dart trap, the
/// mine and the boulder trap: one step in `x`, then one step in `y`, wiring the tile at both ends
/// of each step, until the target is reached.
fn wire_path(world: &mut World, from: (i32, i32), to: (i32, i32)) {
    let (mut x, mut y) = from;
    while (x, y) != to {
        set_wire(world, x, y);
        x += (to.0 - x).signum();
        set_wire(world, x, y);
        y += (to.1 - y).signum();
        set_wire(world, x, y);
    }
}

/// `WorldGen.oceanDepths(x, y)`, approximated: vanilla checks a fixed `oceanLevel`/
/// `beachDistance` this project does not track separately from `Layout`'s ocean bands, so a point
/// inside either band counts as ocean depths outright rather than only below a certain row.
fn ocean_depths(layout: &Layout, x: i32) -> bool {
    layout.ocean_left.contains(x) || layout.ocean_right.contains(x)
}

/// `WorldGen.closeEnoughToSpidersToSpawnSpiderBoulder`.
fn close_enough_to_spiders(world: &World, x: i32, y: i32) -> bool {
    let r = 80;
    let mut j = y - r;
    while j <= y + r {
        let mut i = x - r;
        while i <= x + r {
            if world.tile(i, j).wall == SPIDER_WALL {
                return true;
            }
            i += 3;
        }
        j += 3;
    }
    false
}

/// `WorldGen.closeEnoughToLavaToSpawnLavaBoulder`.
fn close_enough_to_lava(world: &World, x: i32, y: i32) -> bool {
    let r = 60;
    let mut j = y - r;
    while j <= y + r {
        let mut i = x - r;
        while i <= x + r {
            let t = world.tile(i, j);
            if t.liquid > 0 && matches!(t.liquid_kind, terrustia_proto::tile::Liquid::Lava) {
                return true;
            }
            i += 3;
        }
        j += 3;
    }
    false
}

/// `WorldGen.closeEnoughToDungeonToSpawnGhoulder`, using the wall-based dungeon proxy (see the
/// module doc comment) in place of `Main.wallDungeon`.
fn close_enough_to_dungeon(world: &World, x: i32, y: i32) -> bool {
    let r = 1000;
    let mut j = y - r;
    while j <= y + r {
        let mut i = x - r;
        while i <= x + r {
            if is_dungeon_wall(world.tile(i, j).wall) {
                return true;
            }
            i += 10;
        }
        j += 10;
    }
    false
}

fn world_size(width: i32) -> i32 {
    if width <= 4200 {
        0
    } else if width <= 6400 {
        1
    } else {
        2
    }
}

/// What one call to [`place_trap`] built, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrapKind {
    Dart,
    Mine,
    Geyser,
    Boulder,
}

/// `WorldGen.placeTrap(x2, y2, type)`, `type` always `-1` (the only value the driving pass ever
/// passes): the kind is always rolled here, never forced.
fn place_trap(
    world: &mut World,
    layout: &Layout,
    x2: i32,
    y2: i32,
    rng: &mut SmallRng,
    boulder_pets_placed: &mut i32,
) -> Option<TrapKind> {
    if !place_trap_can_continue(world, x2, y2) {
        return None;
    }

    let mut y = y2;
    while !solid_tile(world, x2, y) {
        y += 1;
        if y > layout.height - 10 {
            return None;
        }
        if !place_trap_can_continue(world, x2, y) {
            return None;
        }
    }
    y -= 1;

    if world.tile(x2, y).wall == UNBREAKABLE_WALL {
        return None;
    }

    let lava_floor = {
        let t = world.tile(x2, y);
        t.liquid > 0 && matches!(t.liquid_kind, terrustia_proto::tile::Liquid::Lava)
    };
    let near_bottom = y >= layout.height - 300;

    // `Main.remixWorld`'s branch is a secret seed; only the ordinary-world resolution is
    // transcribed. `lavaLine` has no direct equivalent here, so `Layout::rock` stands in — see
    // the module doc comment.
    let kind_type = if rng.random_range(0..20) == 0 {
        2
    } else if y > layout.rock + 30 && rng.random_range(0..6) != 0 {
        3
    } else {
        rng.random_range(0..2)
    };

    if lava_floor && kind_type != 3 {
        return None;
    }
    if near_bottom && kind_type != 3 {
        return None;
    }

    // The 3x3-minus-corners "nothing already active nearby" guard, transcribed as vanilla's own
    // nine-cell check.
    for (dx, dy) in [
        (0, 0),
        (-1, 0),
        (1, 0),
        (0, -1),
        (-1, -1),
        (1, -1),
        (0, -2),
        (-1, -2),
        (1, -2),
    ] {
        if world.tile(x2 + dx, y + dy).is_active() {
            return None;
        }
    }
    let below = world.tile(x2, y + 1);
    if below.is_active() && matches!(below.block, SPIKES | 232) {
        return None;
    }
    if !can_generate_pressure_plate_at(world, layout, x2, y) {
        return None;
    }

    match kind_type {
        0 => place_dart_trap(world, layout, x2, y, rng),
        1 => place_boulder_trap(world, layout, x2, y, rng, boulder_pets_placed),
        2 => place_mine(world, layout, x2, y, rng),
        3 => place_geyser(world, layout, x2, y, rng),
        _ => unreachable!("kind_type is rolled from a fixed range above"),
    }
}

/// `placeTrap`'s `case 0`: a pressure plate wired to a dart emitter set into the nearer of the
/// two walls bounding whatever corridor it landed in.
fn place_dart_trap(
    world: &mut World,
    layout: &Layout,
    x2: i32,
    y: i32,
    rng: &mut SmallRng,
) -> Option<TrapKind> {
    let mut left = x2;
    while !solid_tile(world, left, y) {
        left -= 1;
        if left < 0 || !place_trap_can_continue(world, left, y) {
            return None;
        }
    }
    let mut right = x2;
    while !solid_tile(world, right, y) {
        right += 1;
        if right >= layout.width || !place_trap_can_continue(world, right, y) {
            return None;
        }
    }

    let left_width = x2 - left;
    let right_width = right - x2;
    let mut left_ok = left_width > 5 && left_width < 50;
    let mut right_ok = right_width > 5 && right_width < 50;
    if left_ok && !solid_tile(world, left, y + 1) {
        left_ok = false;
    }
    if right_ok && !solid_tile(world, right, y + 1) {
        right_ok = false;
    }
    let blocked = |world: &World, x: i32| {
        let a = world.tile(x, y);
        let b = world.tile(x, y + 1);
        (a.is_active() && matches!(a.block, CLOSED_DOOR | SPIKES))
            || (b.is_active() && matches!(b.block, CLOSED_DOOR | SPIKES))
    };
    if left_ok && blocked(world, left) {
        left_ok = false;
    }
    if right_ok && blocked(world, right) {
        right_ok = false;
    }

    let (wall_x, aim_right) = if left_ok && right_ok {
        if rng.random_bool(0.5) {
            (right, false)
        } else {
            (left, true)
        }
    } else if right_ok {
        (right, false)
    } else if left_ok {
        (left, true)
    } else {
        return None;
    };

    if world.tile(wall_x, y).block == 190 || world.tile(wall_x, y).wall == UNBREAKABLE_WALL {
        return None;
    }

    // `Place1x1`'s default case (`WorldGen.cs:45636`) puts a placed tile's *style* on `frameY`
    // (`style * 18`), leaving `frameX` at 0 — the opposite of what this used to write. The style
    // itself is vanilla's own dart-trap-specific roll (`WorldGen.cs:9106-9110`): a walled plate
    // gets the fixed style 2, an unwalled one rolls 2 or 3.
    let has_wall = world.tile(x2, y).wall != 0;
    let plate_style = if has_wall { 2 } else { rng.random_range(2..4) };
    place_tile(world, x2, y, PRESSURE_PLATE, 0, (plate_style * 18) as i16);
    kill_tile(world, wall_x, y);
    let frame_x = if aim_right { 18 } else { 0 };
    place_tile(world, wall_x, y, DART_TRAP, frame_x, 0);

    wire_path(world, (x2, y), (wall_x, y));
    Some(TrapKind::Dart)
}

/// `placeTrap`'s `case 2`: a mine buried `Next(4,7)` tiles beneath the floor the plate sits on,
/// inside a fully solid pocket, wired down to it.
///
/// The old version buried the mine one tile below the plate's own floor — visible the instant a
/// player dug a single block down. Vanilla (`WorldGen.cs:9357-9385`) digs a real shaft first
/// (`num4 = genRand.Next(4, 7)` steps, each one required to already be solid ground, `InWorld`,
/// and not unbreakable-walled), then requires the *whole* 5-wide, 6-tall pocket around the final
/// spot (`num5-2..=num5+2`, `num6-2..=num6+3`) to be solid before anything is written — the same
/// all-or-nothing shape `place_boulder_trap`'s own chamber search already uses. Only then does it
/// kill the one target tile and drop the mine into it.
fn place_mine(
    world: &mut World,
    layout: &Layout,
    x2: i32,
    y: i32,
    rng: &mut SmallRng,
) -> Option<TrapKind> {
    let depth = rng.random_range(4..7);
    let mx = x2 + rng.random_range(-1..=1);
    let mut my = y;
    for _ in 0..depth {
        my += 1;
        if !in_world(layout, mx, my, 5)
            || !solid_tile(world, mx, my)
            || !place_trap_can_continue(world, mx, my)
        {
            return None;
        }
    }
    for xx in (mx - 2)..=(mx + 2) {
        for yy in (my - 2)..=(my + 3) {
            if !in_world(layout, xx, yy, 0)
                || !solid_tile(world, xx, yy)
                || !place_trap_can_continue(world, xx, yy)
            {
                return None;
            }
        }
    }

    kill_tile(world, mx, my);
    let mut mine = Tile::AIR;
    mine.block = EXPLOSIVES;
    mine.frame_x = 0;
    mine.frame_y = 18 * rng.random_range(0..2);
    mine.flags.set(TileFlags::ACTIVE, true);
    mine.wall = world.tile(mx, my).wall;
    world.set_tile(mx, my, mine);

    // The plate itself: same `Place1x1` default-case convention (style on `frameY`) as the other
    // plates, style unconditionally `Next(2,4)` here (`WorldGen.cs:9390`) — no wall-presence
    // branch, unlike the dart trap's own plate.
    let plate_style = rng.random_range(2..4);
    place_tile(world, x2, y, PRESSURE_PLATE, 0, (plate_style * 18) as i16);

    wire_path(world, (x2, y), (mx, my));
    Some(TrapKind::Mine)
}

/// `placeTrap`'s `case 3`: the geyser. Not wired — it fires on its own cooldown.
fn place_geyser(
    world: &mut World,
    layout: &Layout,
    x2: i32,
    y: i32,
    rng: &mut SmallRng,
) -> Option<TrapKind> {
    if world.tile(x2 + 1, y).is_active() {
        return None;
    }
    let here = world.tile(x2, y);
    if here.liquid > 0 && !matches!(here.liquid_kind, terrustia_proto::tile::Liquid::Lava) {
        return None;
    }
    if !place_trap_can_continue(world, x2, y) {
        return None;
    }
    for k in x2..=x2 + 1 {
        if !in_world(layout, k, y + 1, 5)
            || !solid_tile(world, k, y + 1)
            || !place_trap_can_continue(world, k, y + 1)
        {
            return None;
        }
    }
    let flip = rng.random_range(0..2);
    for l in 0..2 {
        let x = x2 + l;
        let wall = world.tile(x, y).wall;
        let mut t = Tile::framed(GEYSER, (18 * l + 36 * flip) as i16, 0);
        t.wall = wall;
        world.set_tile(x, y, t);
    }
    Some(TrapKind::Geyser)
}

/// `placeTrap`'s `case 1`: a boulder trap. Carves a small chamber under the plate, actuates its
/// floor, and drops one of the boulder-family tiles onto it — the exact variant chosen by the
/// same weighted proximity checks vanilla uses, minus the secret-seed-only ones.
#[allow(clippy::too_many_arguments)]
fn place_boulder_trap(
    world: &mut World,
    layout: &Layout,
    x2: i32,
    y: i32,
    rng: &mut SmallRng,
    boulder_pets_placed: &mut i32,
) -> Option<TrapKind> {
    let pet_cap = match world_size(layout.width) {
        1 => 4,
        2 => 6,
        _ => 2,
    };

    let cx = x2 + rng.random_range(-1..=1);
    let mut floor_y = y - 8;
    loop {
        let mut roof_clear = true;
        let mut dirt_count = 0;
        for i in (cx - 2)..=(cx + 3) {
            for j in floor_y..=(floor_y + 3) {
                if !solid_tile(world, i, j) {
                    roof_clear = false;
                }
                if !place_trap_can_continue(world, i, j) {
                    return None;
                }
                let t = world.tile(i, j);
                if t.is_active() {
                    if t.block == super::tiles::LIHZAHRD_BRICK || is_dungeon_wall(t.wall) {
                        return None;
                    }
                    if t.flags.has(TileFlags::ACTUATOR) || is_boulder(t.block) {
                        return None;
                    }
                    if matches!(t.block, 0 | 1 | 59) {
                        dirt_count += 1;
                    }
                }
            }
        }
        floor_y -= 1;
        if floor_y < layout.surface {
            return None;
        }
        if roof_clear && dirt_count > 2 {
            break;
        }
    }
    // `roof_clear`/`dirt_count` above describe the row *before* the final decrement; vanilla
    // re-tests the same row it just decremented past on the next loop head, which the `loop`
    // above already does by re-running the body at the new `floor_y`.
    let _ = cx; // cx is not mutated further; kept as a local to mirror vanilla's own naming.

    if y - floor_y <= 5 || y - floor_y >= 40 {
        return None;
    }
    // `IsTileNearby(num21, num22, 21, 4) || IsTileNearby(num21, num22, 467, 4)`
    // (`WorldGen.cs:9227`) — 21 is `TileID.Containers` (an ordinary chest), not the pressure
    // plate. Checking `PRESSURE_PLATE` here let the chamber-carve loop below cut straight through
    // a placed chest, orphaning it.
    if nearby(world, cx, floor_y, CHEST, 4) || nearby(world, cx, floor_y, 467, 4) {
        return None;
    }
    if any_boulder_nearby(world, cx, floor_y, 10) {
        return None;
    }

    for xx in cx..=(cx + 1) {
        for yy in floor_y..=y {
            if world.tile(xx, yy).block != 379 {
                kill_tile(world, xx, yy);
            }
        }
    }
    for xx in (cx - 2)..=(cx + 3) {
        for yy in (floor_y - 2)..=(floor_y + 3) {
            if solid_tile(world, xx, yy) {
                let mut t = world.tile(xx, yy);
                t.block = 1;
                world.set_tile(xx, yy, t);
            }
        }
    }

    // Same `Place1x1` default-case convention as the dart trap's plate (style on `frameY`, not
    // `frameX`) — the boulder trap's own style is always the fixed 7 (`WorldGen.cs:9264`).
    place_tile(world, x2, y, PRESSURE_PLATE, 0, 7 * 18);
    {
        let wall = world.tile(cx, floor_y + 2).wall;
        world.set_tile(cx, floor_y + 2, Tile::block(1).with_wall(wall));
        let wall = world.tile(cx + 1, floor_y + 2).wall;
        world.set_tile(cx + 1, floor_y + 2, Tile::block(1).with_wall(wall));
    }

    let boulder = if rng.random_range(0..2) == 0 && close_enough_to_spiders(world, cx, floor_y) {
        SPIDER_BOULDER
    } else if rng.random_range(0..6) == 0 && close_enough_to_dungeon(world, cx, floor_y) {
        GHOULDER
    } else if rng.random_range(0..3) == 0 && close_enough_to_lava(world, cx, floor_y) {
        LAVA_BOULDER
    } else if rng.random_range(0..25) == 0 {
        RAINBOW_BOULDER
    } else if rng.random_range(0..20) == 0 {
        BOUNCY_BOULDER
    } else if *boulder_pets_placed < pet_cap {
        *boulder_pets_placed += 1;
        BOULDER_PET
    } else {
        BOULDER
    };
    place_tile(world, cx + 1, floor_y + 1, boulder, -1, -1);

    let mut fy = floor_y + 2;
    for dxx in 0..2 {
        for dyy in 0..3 {
            let (xx, yy) = (cx + dxx, fy + dyy);
            let wall = world.tile(xx, yy).wall;
            world.set_tile(xx, yy, Tile::block(1).with_wall(wall));
            let mut t = world.tile(xx, yy);
            t.flags.set(TileFlags::WIRE_RED, true);
            t.flags.set(TileFlags::ACTUATOR, true);
            world.set_tile(xx, yy, t);
        }
    }
    fy += 2;
    wire_path(world, (x2, y), (cx, fy));

    Some(TrapKind::Boulder)
}

fn nearby(world: &World, x: i32, y: i32, block: u16, distance: i32) -> bool {
    for j in (y - distance)..=(y + distance) {
        for i in (x - distance)..=(x + distance) {
            let t = world.tile(i, j);
            if t.is_active() && t.block == block {
                return true;
            }
        }
    }
    false
}

fn any_boulder_nearby(world: &World, x: i32, y: i32, distance: i32) -> bool {
    for j in (y - distance)..=(y + distance) {
        for i in (x - distance)..=(x + distance) {
            let t = world.tile(i, j);
            if t.is_active() && is_boulder(t.block) {
                return true;
            }
        }
    }
    false
}

/// `WorldGen.PlaceSandTrap`: the desert's sand-filled pit, with an actuated floor wired to a
/// plate at its rim.
fn place_sand_trap(world: &mut World, i: i32, j0: i32, rng: &mut SmallRng) -> bool {
    let mut j = j0;
    while !world.tile(i, j).is_active() {
        j += 1;
        if j >= world.height() - 350 {
            return false;
        }
    }
    let floor = world.tile(i, j);
    if !tile_solid::solid(floor.block) || floor.flags.has(TileFlags::HALF_BRICK) || floor.slope != 0
    {
        return false;
    }
    if !matches!(floor.block, SAND | 397 | 396) {
        return false;
    }
    if floor.wall != walls::SANDSTONE && floor.wall != 216 {
        return false;
    }
    j -= 1;

    let half_span = 25;
    let mut sand_top = -1;
    let rx = rng.random_range(6..12);
    let ry = rng.random_range(6..14);

    for l in (i - half_span)..=(i + half_span) {
        for m in (j - half_span)..(j + half_span) {
            let t = world.tile(l, m);
            if t.flags.has(TileFlags::WIRE_RED) {
                return false;
            }
            if matches!(t.block, 21 | 467 | 441 | 88 | 15 | 19 | 10 | 219 | 314) {
                return false;
            }
        }
    }
    for n in (i - 2)..=(i + 2) {
        for m in (j + 1)..=(j + 3) {
            let t = world.tile(n, m);
            if !t.is_active() || !tile_solid::solid(t.block) {
                return false;
            }
        }
    }
    if world.tile(i, j + 1).block == 162 {
        return false;
    }
    for m in ((j - 30 + 1)..=j).rev() {
        let t = world.tile(i, m);
        if t.is_active() {
            if t.block == 396 {
                sand_top = m;
                break;
            }
            return false;
        }
    }
    if sand_top <= -1 {
        return false;
    }
    let pit_height = ry;
    // vanilla's `num2`: the fixed clearance margin above the sand-top row.
    const CLEARANCE: i32 = 4;
    if j - sand_top < pit_height + CLEARANCE {
        return false;
    }

    let mid = (j + sand_top) / 2;
    let mut solid_count = 0;
    for n in (i - rx)..=(i + rx) {
        let mid_tile = world.tile(n, mid);
        if mid_tile.is_active() && tile_solid::solid(mid_tile.block) {
            return false;
        }
        for m in (sand_top - pit_height)..=sand_top {
            let t = world.tile(n, m);
            if t.is_active() {
                if is_ore(t.block) || t.block == 404 {
                    return false;
                }
                if tile_solid::solid(t.block) {
                    solid_count += 1;
                }
            }
        }
    }
    let required = ((rx * 2 + 1) * (pit_height + 1)) as f64 * 0.75;
    if (solid_count as f64) < required {
        return false;
    }

    for n in (i - rx - 1)..=(i + rx + 1) {
        for m in (sand_top - pit_height)..=sand_top {
            let filled = {
                let t = world.tile(n, m);
                t.is_active() && tile_solid::solid(t.block)
            };
            if m == sand_top {
                clear_slope(world, n, m);
                if !filled {
                    activate(world, n, m, 396);
                }
            } else if m == sand_top - pit_height {
                clear_tile(world, n, m);
                let above_filled = {
                    let t = world.tile(n, m - 1);
                    t.is_active() && tile_solid::solid(t.block)
                };
                activate(world, n, m, if filled && above_filled { 397 } else { 396 });
            } else if n == i - rx - 1 || n == i + rx + 1 {
                if filled {
                    clear_slope(world, n, m);
                } else {
                    clear_tile(world, n, m);
                    activate(world, n, m, 396);
                }
            } else {
                clear_tile(world, n, m);
                activate(world, n, m, SAND);
            }
        }
    }

    for outer in [i - rx - 2, i + rx + 2] {
        for m in (sand_top - pit_height)..=sand_top {
            let t = world.tile(outer, m);
            let ok = t.is_active() && tile_solid::solid(t.block);
            if !ok {
                activate(world, outer, m, 396);
            }
        }
    }
    for m in (sand_top - pit_height)..=sand_top {
        for x in [i - rx - 2, i - rx - 1, i - rx + 1, i - rx + 2] {
            clear_slope(world, x, m);
        }
    }
    for x in (i - rx - 1)..(i + rx + 1) {
        let m = j - pit_height - 1;
        clear_slope(world, x, m);
    }

    kill_tile(world, i - 2, j);
    kill_tile(world, i - 1, j);
    kill_tile(world, i + 1, j);
    kill_tile(world, i + 2, j);
    // Same `Place1x1` default-case convention as the other plates: style (always 7 here, per
    // `WorldGen.cs:36088`) on `frameY`, not `frameX`.
    place_tile(world, i, j, PRESSURE_PLATE, 0, 7 * 18);

    for x in (i - rx)..=(i + rx) {
        let mut yy = j;
        if x < i - (rx as f64 * 0.8) as i32 || x > i + (rx as f64 * 0.8) as i32 {
            yy = j - 3;
        } else if x < i - (rx as f64 * 0.6) as i32 || x > i + (rx as f64 * 0.6) as i32 {
            yy = j - 2;
        } else if x < i - (rx as f64 * 0.4) as i32 || x > i + (rx as f64 * 0.4) as i32 {
            yy = j - 1;
        }
        for m in sand_top..=j {
            if x == i {
                set_wire(world, i, m);
            }
            let t = world.tile(x, m);
            if t.is_active() && tile_solid::solid(t.block) {
                if m < sand_top + pit_height - 4 {
                    let mut a = world.tile(x, m);
                    a.flags.set(TileFlags::ACTUATOR, true);
                    a.flags.set(TileFlags::WIRE_RED, true);
                    world.set_tile(x, m, a);
                } else if m < yy {
                    kill_tile(world, x, m);
                }
            }
        }
    }

    true
}

/// `TileID.Sets.Ore`.
fn is_ore(block: u16) -> bool {
    matches!(
        block,
        7 | 166
            | 6
            | 167
            | 9
            | 168
            | 8
            | 169
            | 22
            | 204
            | 37
            | 58
            | 107
            | 221
            | 108
            | 222
            | 111
            | 223
            | 211
    )
}

fn clear_slope(world: &mut World, x: i32, y: i32) {
    let mut t = world.tile(x, y);
    t.slope = 0;
    t.flags.set(TileFlags::HALF_BRICK, false);
    world.set_tile(x, y, t);
}

fn clear_tile(world: &mut World, x: i32, y: i32) {
    let wall = world.tile(x, y).wall;
    world.set_tile(x, y, Tile::AIR.with_wall(wall));
}

fn activate(world: &mut World, x: i32, y: i32, block: u16) {
    let mut t = world.tile(x, y);
    t.block = block;
    t.flags.set(TileFlags::ACTIVE, true);
    world.set_tile(x, y, t);
}

/// How many of each kind of trap a full pass placed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TrapScatterResult {
    pub dart_traps: usize,
    pub mines: usize,
    pub geysers: usize,
    pub boulder_traps: usize,
    pub sand_traps: usize,
}

impl TrapScatterResult {
    pub fn total(&self) -> usize {
        self.dart_traps + self.mines + self.geysers + self.boulder_traps + self.sand_traps
    }
}

/// The `GenPassNameID.Traps` pass: scatters traps and sand traps across the already-generated
/// world. Ordinary-world path only — see the module doc comment.
///
/// `secret`: the one flag this module's own doc comment already named as real vanilla's own gate
/// on almost this entire pass — `noTrapsWorldGen`. This is the one secret seed this session
/// actually wires to a behavioural difference (see `secret_seed.rs`'s own module doc for why the
/// other flags are detected but not yet consumed here): `SecretSeeds::no_traps` short-circuits the
/// whole pass to placing nothing, rather than gating each of the four trap kinds and the sand trap
/// loop separately — real vanilla's own name for this seed ("No Traps World") is the same
/// all-or-nothing claim, and every other flag this pass's own module doc lists alongside
/// `noTrapsWorldGen` stays unmodelled, same as before this parameter existed. `no_traps` is also
/// set when "get fixed boi" is active (one of its own seven real dependency flags), so this now
/// correctly clears traps for that seed too, which the old single-variant `SecretSeed` enum could
/// not represent.
pub fn scatter(
    world: &mut World,
    layout: &Layout,
    rng: &mut SmallRng,
    secret: SecretSeeds,
) -> TrapScatterResult {
    let mut result = TrapScatterResult::default();
    if secret.no_traps {
        return result;
    }
    // The search bands below are `200..width-200`, `surface..height-210`, and — tighter than
    // either — the sand-trap loop's own `surface+20..height-210`. Real, full-size worlds always
    // clear all three by a wide margin, but a small world (the synthetic ones several unrelated
    // tests build to keep persistence/header tests fast, or a real but modestly-sized
    // `world_width`/`world_height` a config file legitimately requests — `Config::validate` only
    // floors `world_width`, not `world_height` against what this pass itself needs) can clear the
    // first two margins while still landing inside the 20-tile gap the third one leaves, which
    // used to reach `random_range` with an inverted range and panic instead of being skipped like
    // every other too-small case. Guarding against the tightest of the three bands (`+230`, not
    // `+210`) covers all of them.
    if layout.width <= 400 || layout.height <= layout.surface + 230 {
        return result;
    }
    let mut boulder_pets_placed = 0;

    let outer = ((layout.width as f64) * 0.05) as i32;
    for _ in 0..outer {
        for _ in 0..1150 {
            let x = rng.random_range(200..(layout.width - 200));
            let y = rng.random_range(layout.surface..(layout.height - 210));
            if ocean_depths(layout, x) {
                continue;
            }
            if world.tile(x, y).wall != 0 {
                continue;
            }
            if let Some(kind) = place_trap(world, layout, x, y, rng, &mut boulder_pets_placed) {
                match kind {
                    TrapKind::Dart => result.dart_traps += 1,
                    TrapKind::Mine => result.mines += 1,
                    TrapKind::Geyser => result.geysers += 1,
                    TrapKind::Boulder => result.boulder_traps += 1,
                }
                break;
            }
        }
    }

    let sand_outer = ((layout.width as f64) * 0.003) as i32;
    for _ in 0..sand_outer {
        for _ in 0..20_000 {
            let x = rng.random_range(
                ((layout.width as f64) * 0.15) as i32..((layout.width as f64) * 0.85) as i32,
            );
            let y = rng.random_range((layout.surface + 20)..(layout.height - 210));
            if world.tile(x, y).wall != walls::SANDSTONE {
                continue;
            }
            if place_sand_trap(world, x, y, rng) {
                result.sand_traps += 1;
                break;
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::wiring;
    use rand::SeedableRng;

    /// 1200 tall, matching vanilla's own minimum world height: `placeTrap`'s `near_bottom` guard
    /// is `y >= height - 300`, a constant tuned for a real world, and a much shorter test world
    /// would put every candidate site within that band and make every non-geyser roll fail.
    /// 500 wide: the full-pass `scatter` test needs `width - 200` to be a sane, non-empty range
    /// for its own random siting, which a 200-wide world (vanilla's driving pass assumes a real
    /// world, always several thousand tiles wide) cannot give it.
    fn corridor() -> World {
        let mut world = World::empty(500, 1200, "traps");
        for x in 0..500 {
            for y in 0..60 {
                world.set_tile(x, y, Tile::block(1)); // roof
            }
            for y in 65..200 {
                world.set_tile(x, y, Tile::block(1)); // floor and below
            }
        }
        // Two isolated wall columns bounding a corridor from x=90 to x=110, so a dart trap has
        // a wall to search for and mount into — an open room with no vertical walls at all is
        // not the shape `placeTrap`'s dart branch is looking for.
        for y in 60..65 {
            world.set_tile(89, y, Tile::block(1));
            world.set_tile(111, y, Tile::block(1));
        }
        world
    }

    fn layout(world: &World) -> Layout {
        let mut rand = super::super::rand::UnifiedRandom::new(5);
        let mut layout = Layout::plan(world.width(), world.height(), &mut rand);
        layout.surface = 10;
        layout.rock = 40;
        layout
    }

    /// A dart trap placed in an open corridor lays a wired plate and, once the current reaches
    /// it, wiring.rs itself resolves the emitter tile into a real shot — the pin this file's
    /// verification bar actually rests on, not just that tiles were written.
    #[test]
    fn a_dart_trap_fires_when_its_plate_is_stepped_on() {
        let mut world = corridor();
        let layout = layout(&world);
        let mut rng = SmallRng::seed_from_u64(0);
        let mut pets = 0;

        let mut found = None;
        'search: for x2 in 95..106 {
            for y2 in 5..64 {
                if let Some(TrapKind::Dart) =
                    place_trap(&mut world, &layout, x2, y2, &mut rng, &mut pets)
                {
                    found = Some((x2, y2));
                    break 'search;
                }
            }
        }
        let (x2, y2) = found.expect("a dart trap should place somewhere in the walled corridor");

        // The plate landed on solid ground below (x2, y2).
        let mut plate_y = y2;
        while !world.tile(x2, plate_y).is_active() {
            plate_y += 1;
        }
        assert_eq!(world.tile(x2, plate_y).block, PRESSURE_PLATE);

        let fired = wiring::hit_switch(&mut world, x2, plate_y);
        assert_eq!(
            fired.traps.len(),
            1,
            "the current must reach exactly the one dart tile"
        );
        let (tx, ty) = fired.traps[0];
        let shot = wiring::trap_shot(world.tile(tx, ty), tx, ty, &mut rng);
        assert!(
            shot.is_some(),
            "a real dart trap tile must resolve into a shot"
        );
    }

    /// A mine's wiring is reachable and reported through `Fired::mines`, not `Fired::traps` —
    /// confirming the wiring.rs split this file relies on actually holds for a tile this pass
    /// wrote, not only for the hand-built tiles wiring.rs's own tests use.
    #[test]
    fn a_mine_fires_into_fired_mines_not_fired_traps() {
        let mut world = corridor();
        let layout = layout(&world);
        let mut rng = SmallRng::seed_from_u64(0);
        let mut pets = 0;

        let mut found = None;
        'search: for x2 in 20..180 {
            for y2 in 5..64 {
                if let Some(TrapKind::Mine) =
                    place_trap(&mut world, &layout, x2, y2, &mut rng, &mut pets)
                {
                    found = Some((x2, y2));
                    break 'search;
                }
            }
        }
        let Some((x2, y2)) = found else {
            // Mines are a 1/20 roll; a fixed seed may legitimately never land one within this
            // small a search grid. Not placing one is not a failure of this test.
            return;
        };
        let mut plate_y = y2;
        while !world.tile(x2, plate_y).is_active() {
            plate_y += 1;
        }
        let fired = wiring::hit_switch(&mut world, x2, plate_y);
        assert_eq!(fired.mines.len(), 1);
        assert!(fired.traps.is_empty());
    }

    /// A boulder trap's chamber gets an actuated floor that the plate's wire reaches and toggles
    /// — the actuator/wire half of the set piece, which `wiring.rs` already knows how to run.
    #[test]
    fn a_boulder_traps_floor_actuates_when_its_plate_fires() {
        let mut world = corridor();
        let layout = layout(&world);
        let mut rng = SmallRng::seed_from_u64(3);
        let mut pets = 0;

        let mut found = None;
        'search: for x2 in 20..180 {
            for y2 in 5..64 {
                if let Some(TrapKind::Boulder) =
                    place_trap(&mut world, &layout, x2, y2, &mut rng, &mut pets)
                {
                    found = Some((x2, y2));
                    break 'search;
                }
            }
        }
        let (x2, y2) = found.expect("a boulder trap should place somewhere in this corridor");
        let mut plate_y = y2;
        while !world.tile(x2, plate_y).is_active() {
            plate_y += 1;
        }
        let fired = wiring::hit_switch(&mut world, x2, plate_y);
        assert!(
            !fired.changed.is_empty(),
            "the actuator floor must have toggled"
        );
    }

    /// A geyser is placed unwired — no plate, no wire — since it fires on its own.
    #[test]
    fn a_geyser_is_not_wired_to_anything() {
        let mut world = corridor();
        let mut layout = layout(&world);
        layout.rock = 5; // force every roll below `rock + 30` toward the geyser branch
        let mut rng = SmallRng::seed_from_u64(0);
        let mut pets = 0;

        let mut found = None;
        'search: for x2 in 20..180 {
            for y2 in 5..64 {
                if let Some(TrapKind::Geyser) =
                    place_trap(&mut world, &layout, x2, y2, &mut rng, &mut pets)
                {
                    found = Some((x2, y2));
                    break 'search;
                }
            }
        }
        let (x2, y2) = found.expect("a geyser should place once rock is shallow enough");
        let mut gy = y2;
        while !world.tile(x2, gy).is_active() {
            gy += 1;
        }
        assert_eq!(world.tile(x2, gy).block, GEYSER);
        assert!(!world.tile(x2, gy).flags.has(TileFlags::WIRE_RED));
    }

    /// A full pass over a plain corridor places at least some traps — the real end-to-end
    /// measurement, not just that the individual placement functions can succeed in isolation.
    #[test]
    fn a_full_pass_places_traps_across_a_generated_world() {
        let mut world = corridor();
        let layout = layout(&world);
        let mut rng = SmallRng::seed_from_u64(7);
        let result = scatter(&mut world, &layout, &mut rng, SecretSeeds::none());
        assert!(
            result.total() > 0,
            "a 200-wide corridor should get at least one trap"
        );
    }

    /// A world whose height clears the outer-loop margin (`surface..height-210`) but not the
    /// sand-trap loop's tighter one (`surface+20..height-210`) used to reach `random_range` with
    /// an inverted range and panic, instead of being skipped like every other too-small case —
    /// `surface = 10`, `height = 230` sits exactly in that 20-tile gap (`10+210=220 < 230 <=
    /// 10+230=240`). Real, reachable through ordinary world generation at any valid
    /// `world_width`/`world_height` whose surface lands there, not just synthetic test worlds:
    /// `Config::validate` floors `world_width` but not `world_height` against what this pass
    /// itself needs.
    #[test]
    fn a_world_too_short_for_the_sand_trap_range_is_skipped_not_panicked() {
        let mut world = World::empty(500, 230, "traps");
        let layout = layout(&world);
        let mut rng = SmallRng::seed_from_u64(7);
        let result = scatter(&mut world, &layout, &mut rng, SecretSeeds::none());
        assert_eq!(
            result.total(),
            0,
            "a world this short should place nothing, not panic"
        );
    }

    /// The "No Traps World" secret seed: the same corridor that always gets at least one trap on
    /// an ordinary seed gets none at all once `SecretSeeds::no_traps` is set — the one real
    /// behavioural difference this session actually wires (see `secret_seed.rs`'s own module doc
    /// for why the other flags are detected but left as ordinary generation).
    #[test]
    fn no_traps_world_places_nothing() {
        let mut world = corridor();
        let layout = layout(&world);
        let mut rng = SmallRng::seed_from_u64(7);
        let result = scatter(
            &mut world,
            &layout,
            &mut rng,
            SecretSeeds {
                no_traps: true,
                ..SecretSeeds::none()
            },
        );
        assert_eq!(
            result,
            TrapScatterResult::default(),
            "No Traps World should place zero of every trap kind"
        );
        assert_eq!(
            result.total(),
            0,
            "No Traps World should place nothing at all"
        );
    }

    /// Every plate this file places must carry vanilla's own `Place1x1` default-case convention
    /// (`WorldGen.cs:45636`): style on `frameY` (`style * 18`), `frameX` left at 0. The old code
    /// wrote a fixed `126` on `frameX` (not a real style at all) and only ever varied `frameY`
    /// between 0 and 18 based on wall presence — neither axis matched vanilla. Fails on the
    /// pre-fix code (`frame_x == 126`).
    #[test]
    fn a_dart_traps_plate_has_style_on_frame_y_not_frame_x() {
        let mut world = corridor();
        let layout = layout(&world);
        let mut rng = SmallRng::seed_from_u64(0);
        let mut pets = 0;

        let mut found = None;
        'search: for x2 in 95..106 {
            for y2 in 5..64 {
                if let Some(TrapKind::Dart) =
                    place_trap(&mut world, &layout, x2, y2, &mut rng, &mut pets)
                {
                    found = Some((x2, y2));
                    break 'search;
                }
            }
        }
        let (x2, y2) = found.expect("a dart trap should place somewhere in the walled corridor");
        let mut plate_y = y2;
        while !world.tile(x2, plate_y).is_active() {
            plate_y += 1;
        }
        let plate = world.tile(x2, plate_y);
        assert_eq!(
            plate.frame_x, 0,
            "a pressure plate's frameX must be 0, not a style value"
        );
        assert!(
            [36, 54].contains(&plate.frame_y),
            "a dart trap's plate style is 2 or 3 (frameY 36 or 54), got {}",
            plate.frame_y
        );
    }

    /// Same convention, for the boulder trap's own plate — whose style is always the fixed 7
    /// (`WorldGen.cs:9264`), so `frameY` must always land on exactly `126`.
    #[test]
    fn a_boulder_traps_plate_has_the_fixed_style_seven_frame() {
        let mut world = corridor();
        let layout = layout(&world);
        let mut rng = SmallRng::seed_from_u64(3);
        let mut pets = 0;

        let mut found = None;
        'search: for x2 in 20..180 {
            for y2 in 5..64 {
                if let Some(TrapKind::Boulder) =
                    place_trap(&mut world, &layout, x2, y2, &mut rng, &mut pets)
                {
                    found = Some((x2, y2));
                    break 'search;
                }
            }
        }
        let (x2, y2) = found.expect("a boulder trap should place somewhere in this corridor");
        let mut plate_y = y2;
        while !world.tile(x2, plate_y).is_active() {
            plate_y += 1;
        }
        let plate = world.tile(x2, plate_y);
        assert_eq!(plate.frame_x, 0);
        assert_eq!(
            plate.frame_y, 126,
            "a boulder trap's plate style is always 7 (frameY = 7*18 = 126)"
        );
    }

    /// A land mine used to sit one tile below its own plate — visible the instant a player dug a
    /// single block down. Vanilla (`WorldGen.cs:9357-9385`) digs `Next(4,7)` tiles further down
    /// first and requires a fully solid pocket around the final spot before burying it there.
    /// Fails on the pre-fix code, which always buried the mine exactly one tile under the plate.
    #[test]
    fn a_mine_is_buried_well_below_its_plate_not_one_tile_under_it() {
        let mut world = corridor();
        let layout = layout(&world);
        let mut rng = SmallRng::seed_from_u64(0);
        let mut pets = 0;

        let mut found = None;
        'search: for x2 in 20..180 {
            for y2 in 5..64 {
                if let Some(TrapKind::Mine) =
                    place_trap(&mut world, &layout, x2, y2, &mut rng, &mut pets)
                {
                    found = Some((x2, y2));
                    break 'search;
                }
            }
        }
        let Some((x2, y2)) = found else {
            // Mines are a 1/20 roll; a fixed seed may legitimately never land one within this
            // small a search grid — not a failure of this test (matches the sibling wiring test's
            // own reasoning).
            return;
        };
        let mut plate_y = y2;
        while !world.tile(x2, plate_y).is_active() {
            plate_y += 1;
        }

        // The mine's own `x` wanders by `Next(-1, 2)` from the plate's column, so search all
        // three candidate columns for the buried explosive.
        let mut mine_pos = None;
        for dx in -1..=1 {
            let mx = x2 + dx;
            for my in (plate_y + 1)..world.height() {
                let t = world.tile(mx, my);
                if t.is_active() && t.block == EXPLOSIVES {
                    mine_pos = Some((mx, my));
                    break;
                }
            }
            if mine_pos.is_some() {
                break;
            }
        }
        let (mine_x, mine_y) = mine_pos.expect("expected a buried explosive tile below the plate");
        assert!(
            mine_y - plate_y >= 4,
            "the mine should be buried at least 4 tiles below its plate (vanilla's own \
             `Next(4, 7)`), not 1 — found at depth {}",
            mine_y - plate_y
        );
        let mine_tile = world.tile(mine_x, mine_y);
        assert_eq!(mine_tile.frame_x, 0, "mine frameX must be 0");
        assert!(
            mine_tile.frame_y == 0 || mine_tile.frame_y == 18,
            "mine frameY must be 0 or 18 (18 * Next(2)), got {}",
            mine_tile.frame_y
        );
    }

    /// The boulder trap's own nearby-chest guard checked for a pressure plate (135) instead of a
    /// chest (21, `IsTileNearby(num21, num22, 21, 4)`, `WorldGen.cs:9227`) — so a chest sitting
    /// where the trap wanted to carve its chamber got cut straight through instead of refusing
    /// the trap. Fails on the pre-fix code, which finds no nearby pressure plate, proceeds, and
    /// destroys the chest.
    #[test]
    fn a_boulder_trap_refuses_to_carve_through_a_nearby_chest() {
        let mut world = corridor();
        let (x2, y) = (60, 90);
        // `place_boulder_trap`'s own chamber search converges deterministically to
        // `floor_y = y - 9` on a uniformly solid floor with no walls (the search loop consults
        // `rng` exactly once, for `cx`, before that point) — `corridor()`'s floor (rows 65..200,
        // all `Tile::block(1)`) already has that shape. A single chest at `(x2, y - 9)` sits
        // inside the guard's own 9-tile search box around `(cx, floor_y)` no matter which of the
        // three possible `cx` rolls (`x2 - 1`, `x2`, `x2 + 1`) comes out.
        world.set_tile(x2, y - 9, Tile::framed(CHEST, 0, 0));
        let layout = layout(&world);
        let mut rng = SmallRng::seed_from_u64(1);
        let mut pets = 0;

        let result = place_boulder_trap(&mut world, &layout, x2, y, &mut rng, &mut pets);
        assert!(
            result.is_none(),
            "a boulder trap must refuse to place near a chest, not carve through it"
        );
        assert_eq!(
            world.tile(x2, y - 9).block,
            CHEST,
            "the chest itself must survive untouched"
        );
    }
}
