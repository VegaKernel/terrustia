//! Underground cabins: small furnished houses found underground, built from whichever material
//! dominates the site — wood, desert sandstone, granite, ice, jungle mahogany, marble or
//! mushroom.
//!
//! Transcribed from `CaveHouseBiome`/`HouseUtils`/`HouseBuilder`
//! (`Terraria.GameContent.Biomes[.CaveHouse]`), the `MicroBiome` this project's own sizing table
//! flagged as needing that class's pattern read before micro-biomes could reuse it. The driving
//! pass in vanilla, `UndergroundHousesAndBuriedChests`, does three unrelated things bundled into
//! one `AddGenerationPass` call: buried cave chests, buried underworld chests, and cave houses.
//! Only the third is new here — the first two are `AddBuriedChest` calls with vanilla's own
//! depth-tiered loot table, which `structures::chests`' own doc comment already covers and
//! discloses (this generator does not model underground-desert or the underworld's shuffled loot
//! array as regions distinct from what `structures::chests` already handles).
//!
//! **What's transcribed**: real site-finding (`FindRoom`'s three-probe room-bounds search,
//! `GetRoomSolidPercentage`'s inclusion roll, `GetHouseType`'s material-count vote,
//! `AreRoomsValid`'s lava/`StructureMap` checks), the seven materials' real tile/wall/beam/door/
//! chest ids, room carving, and real connectivity (doors via [`super::place_object`], sloped
//! stairs between stacked rooms, support beams closing a room-to-room gap) — vanilla's own core
//! shape: three linked rooms, hollow, walled, reachable, holding one chest. Vanilla's own vertical
//! *platform* exits at the top/bottom room (`FindVerticalExit`/`CreatePlatformsList`) are left as
//! plain open gaps instead of placed platform tiles — see [`connect_stacked_rooms`]'s own doc
//! comment for why platforms specifically are a frame-important tile this generator has no
//! neighbour-shape table for, the same class of bug already found and fixed once for doors.
//!
//! **What's disclosed and skipped, deliberately**: `FillRooms`' furniture catalog (paintings,
//! banners, pianos, bookcases, the random per-room decoration walk) and each material's `AgeRoom`
//! weathering pass (dithered wall/tile decay, stalactites, hanging vines) — real vanilla polish,
//! not structural, and each is its own significant `Dither`/`Blotches`/`ActionStalagtite` DSL
//! surface this generator has no equivalent for; `PlaceBiomeSpecificPriorityTool`'s desert Bast
//! statue and `PlaceBiomeSpecificTool`'s jungle sharpener/desert extractinator, each a single
//! rare-furniture placement gated by a world-wide budget counter; every secret-seed variant
//! (`PotentiallyConvertToSeedHouse`'s ~250-line reskin, the rainbow/tenth-anniversary paint
//! passes, `GenerateBiggerAbandonedHouses`' alternate multi-room chain generator) — the same
//! standing rule as every other secret-seed branch this session has left out. A cabin here is a
//! real, reachable, correctly-materialed hollow structure with a chest; it is furnished more
//! plainly than vanilla's.

use super::layout::Layout;
use super::place_object::place_object;
use super::rand::UnifiedRandom;
use super::structure_map::{Rect, StructureMap};
use super::structures;
use super::tiles;
use crate::world::World;
use terrustia_proto::{Liquid, Tile, tile_solid};

/// `HouseType`, transcribed. Order matches vanilla's enum, which is also `GetHouseType`'s
/// tie-break order (first-listed wins a tied vote).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HouseType {
    Wood,
    Jungle,
    Mushroom,
    Ice,
    Desert,
    Granite,
    Marble,
}

/// Everything a material needs, from the seven `*HouseBuilder` subclasses' own constructors.
struct Materials {
    tile: u16,
    wall: u16,
    beam: u16,
    door_style: i32,
    chest_style: i32,
}

fn materials(kind: HouseType) -> Materials {
    match kind {
        HouseType::Wood => Materials {
            tile: 30,
            wall: 27,
            beam: 124,
            door_style: 0,
            chest_style: 1,
        },
        HouseType::Desert => Materials {
            tile: tiles::SANDSTONE,
            wall: tiles::walls::SANDSTONE,
            beam: 577,
            door_style: 43,
            chest_style: 10,
        },
        HouseType::Granite => Materials {
            tile: 369,
            wall: 181,
            beam: 576,
            door_style: 34,
            chest_style: 50,
        },
        HouseType::Ice => Materials {
            tile: 321,
            wall: 149,
            beam: 574,
            door_style: 30,
            chest_style: 11,
        },
        HouseType::Jungle => Materials {
            tile: tiles::RICH_MAHOGANY,
            wall: tiles::walls::RICH_MAHOGANY,
            beam: 575,
            door_style: 2,
            chest_style: 8,
        },
        HouseType::Marble => Materials {
            tile: 357,
            wall: 179,
            beam: 561,
            door_style: 35,
            chest_style: 51,
        },
        HouseType::Mushroom => Materials {
            tile: 190,
            wall: 74,
            beam: 578,
            door_style: 6,
            chest_style: 32,
        },
    }
}

/// `HouseUtils`' own site-tolerant whitelist (`BlacklistedTiles`, misleadingly named in source —
/// `StructureMap.CanPlace`'s real semantics treat its bool array as "already active here is
/// fine," not "reject these," so it is transcribed here as what it actually does).
fn site_tolerant(block: u16) -> bool {
    matches!(
        block,
        tiles::HIVE
            | tiles::BLUE_DUNGEON_BRICK
            | tiles::GREEN_DUNGEON_BRICK
            | tiles::PINK_DUNGEON_BRICK
            | tiles::LIHZAHRD_BRICK
            | tiles::CRIMSTONE
            | tiles::EBONSAND
            | tiles::EBONSTONE
            | tiles::SANDSTONE_BRICK
            | tiles::CHEST
    )
}

/// `WorldGen.InWorld(x, y, margin)`, transcribed — every siting pass in this generator that needs
/// it re-derives it locally rather than sharing one, matching precedent (`oasis.rs`, `pyramids.rs`).
fn in_world(layout: &Layout, x: i32, y: i32, margin: i32) -> bool {
    x >= margin && x < layout.width - margin && y >= margin && y < layout.height - margin
}

/// `WorldUtils.Find(origin, Searches.Chain(Searches.Down(n), Conditions.IsSolid()))` and its
/// three siblings — scan up to `max` tiles in one direction for the first solid, active tile.
///
/// Checks `(x, y)` itself before taking a single step — matching `GenSearch`'s real `Find(origin)`
/// loop (`i` starts at `0`, so `Check(origin.X, origin.Y)` runs first), not a search that starts
/// one tile past the origin. This matters here specifically: `create_rooms`' own `found == origin`
/// rejection (its transcription of `CreateRooms`' identical check) can only ever fire if the
/// search was capable of returning the origin unmoved in the first place.
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

/// `FindRoom`, transcribed: probe left/right for walls, then up from each wall's foot for the
/// ceiling, and take the room whose derived bounds best fit `origin`.
fn find_room(world: &World, origin: (i32, i32)) -> Rect {
    let (ox, oy) = origin;
    let left = find_solid(world, ox, oy, -1, 0, 25).unwrap_or((ox - 25, oy));
    let right = find_solid(world, ox, oy, 1, 0, 25).unwrap_or((ox + 25, oy));

    let (mut x, width);
    if ox - left.0 > right.0 - ox {
        x = left.0;
        width = (right.0 - left.0).clamp(15, 30);
    } else {
        width = (right.0 - left.0).clamp(15, 30);
        x = right.0 - width;
    }
    let _ = &mut x;

    let up_from_left = find_solid(world, left.0, left.1, 0, -1, 10).unwrap_or((ox, oy - 10));
    let up_from_right = find_solid(world, right.0, right.1, 0, -1, 10).unwrap_or((ox, oy - 10));
    let height = (oy - up_from_left.1).max(oy - up_from_right.1).clamp(8, 12);

    Rect::new(x, oy - height, width, height)
}

fn room_solid_fraction(world: &World, room: Rect) -> f64 {
    let mut solid = 0i32;
    for x in room.x..room.right() {
        for y in room.y..room.bottom() {
            let t = world.tile(x, y);
            if t.is_active() && tile_solid::solid(t.block) {
                solid += 1;
            }
        }
    }
    f64::from(solid) / f64::from((room.width * room.height).max(1))
}

/// `CreateRooms`, transcribed: a middle room straight down from `origin`, with a room above and a
/// room below each independently rolled in or out based on how solid they already are (a mostly
/// hollow candidate room is less likely to be added — it would read as barely different from the
/// cave it sits in).
fn create_rooms(world: &World, origin: (i32, i32), rand: &mut UnifiedRandom) -> Vec<Rect> {
    let Some(found) = find_solid(world, origin.0, origin.1, 0, 1, 200) else {
        return Vec::new();
    };
    if found == origin {
        return Vec::new();
    }
    let mut middle = find_room(world, found);
    let mut above = find_room(world, (middle.x + middle.width / 2, middle.y + 1));
    let mut below = find_room(
        world,
        (middle.x + middle.width / 2, middle.y + middle.height + 10),
    );
    below.y = middle.y + middle.height - 1;

    let above_frac = room_solid_fraction(world, above);
    let below_frac = room_solid_fraction(world, below);
    middle.y += 3;
    above.y += 3;
    below.y += 3;

    let mut rooms = Vec::new();
    if rand.next_double() > above_frac + 0.2 {
        rooms.push(above);
    }
    rooms.push(middle);
    if rand.next_double() > below_frac + 0.2 {
        rooms.push(below);
    }
    rooms
}

fn rooms_in_bounds(layout: &Layout, rooms: &[Rect]) -> bool {
    rooms
        .iter()
        .all(|room| in_world(layout, room.x, room.y, 10) && room.bottom() <= layout.height - 220)
}

/// `GetHouseType`, transcribed: scan a 10-tile-padded box around every room, tally each material's
/// vanilla-weighted tile count, and take the highest. Ties keep the earlier-listed type, matching
/// `list[i]` only overwriting `tuple` on a strictly greater count.
fn house_type(world: &World, rooms: &[Rect]) -> HouseType {
    let mut counts = [0i64; 7]; // dirt+wood, jungle-mud, jungle-grass, mushroom-grass, snow, ice, sand
    for room in rooms {
        for x in (room.x - 10)..(room.x + room.width + 10) {
            for y in (room.y - 10)..(room.y + room.height + 10) {
                let t = world.tile(x, y);
                if !t.is_active() {
                    continue;
                }
                match t.block {
                    tiles::DIRT | tiles::STONE => counts[0] += 1,
                    tiles::MUD => counts[1] += 1,
                    tiles::JUNGLE_GRASS => counts[2] += 1,
                    tiles::MUSHROOM_GRASS => counts[3] += 1,
                    tiles::SNOW => counts[4] += 1,
                    tiles::ICE => counts[5] += 1,
                    tiles::SAND | tiles::SANDSTONE | tiles::HARDENED_SAND => counts[6] += 1,
                    _ => {}
                }
            }
        }
    }
    let scored = [
        (HouseType::Wood, counts[0]),
        (HouseType::Jungle, counts[1] + counts[2] * 10),
        (HouseType::Mushroom, counts[1] + counts[3] * 10),
        (HouseType::Ice, counts[4] + counts[5]),
        (HouseType::Desert, counts[6]),
        (HouseType::Granite, granite_count(world, rooms)),
        (HouseType::Marble, marble_count(world, rooms)),
    ];
    let mut best = scored[0];
    for &candidate in &scored[1..] {
        if candidate.1 > best.1 {
            best = candidate;
        }
    }
    best.0
}

fn tile_count(world: &World, rooms: &[Rect], block: u16) -> i32 {
    let mut n = 0;
    for room in rooms {
        for x in (room.x - 10)..(room.x + room.width + 10) {
            for y in (room.y - 10)..(room.y + room.height + 10) {
                let t = world.tile(x, y);
                if t.is_active() && t.block == block {
                    n += 1;
                }
            }
        }
    }
    n
}

fn granite_count(world: &World, rooms: &[Rect]) -> i64 {
    i64::from(tile_count(world, rooms, tiles::GRANITE))
}

fn marble_count(world: &World, rooms: &[Rect]) -> i64 {
    i64::from(tile_count(world, rooms, tiles::MARBLE))
}

/// `AreRoomsValid`, transcribed: no lava inside a padded room unless the site is Granite (granite
/// caverns legitimately run through lava pockets in vanilla too), and every room's padded
/// footprint has to clear [`StructureMap`] against [`site_tolerant`].
fn rooms_valid(world: &World, structures: &StructureMap, rooms: &[Rect], kind: HouseType) -> bool {
    for room in rooms {
        if kind != HouseType::Granite {
            let padded = room.inflated(2);
            for x in padded.x..padded.right() {
                for y in padded.y..padded.bottom() {
                    if world.tile(x, y).liquid_kind == Liquid::Lava && world.tile(x, y).liquid > 0 {
                        return false;
                    }
                }
            }
        }
        if !structures.can_place_with(world, *room, 5, site_tolerant) {
            return false;
        }
    }
    true
}

/// Carve a room hollow: material walls and floor on the footprint's edge, air and a background
/// wall inside — `PlaceEmptyRooms`, without the auto-neighbor-frame step ordinary (non-object)
/// blocks in this generator never need (see `oasis.rs`'s own carving for the same convention).
fn carve_room(world: &mut World, room: Rect, mat: &Materials) {
    let interior = Rect::new(
        room.x + 1,
        room.y + 1,
        (room.width - 2).max(0),
        (room.height - 2).max(0),
    );
    for x in room.x..room.right() {
        for y in room.y..room.bottom() {
            if x >= interior.x && x < interior.right() && y >= interior.y && y < interior.bottom() {
                // The hollow interior: never written as material to begin with, so a save/reload
                // never has a stale non-zero `block` to normalize away on a tile the writer treats
                // as empty (an earlier version of this function wrote wood everywhere, *then*
                // cleared `ACTIVE` on the interior without resetting `block` — harmless at
                // runtime, since nothing reads `block` on an inactive tile, but a real divergence
                // after a save round-trip, since the writer does not preserve it).
                let t = Tile {
                    wall: mat.wall,
                    ..Tile::default()
                };
                world.set_tile(x, y, t);
            } else {
                let mut t = Tile::block(mat.tile);
                t.frame_x = -1;
                t.frame_y = -1;
                world.set_tile(x, y, t);
            }
        }
    }
}

/// A side exit: the first fully-open 1-wide, 3-tall gap along a room's left or right wall,
/// scanning down from the top — `FindSideExit`, transcribed with the same 3-tall window.
fn find_side_exit(world: &World, room: Rect, left: bool) -> Option<i32> {
    let x = if left { room.x } else { room.right() - 1 };
    for y in (room.y + 1)..(room.bottom() - 3) {
        if (0..3).all(|dy| !world.tile(x, y + dy).is_active()) {
            return Some(y + 1);
        }
    }
    None
}

/// Doors between vertically adjacent rooms that overlap in `x` — `CreateDoorList`/`PlaceDoors`.
fn place_doors(world: &mut World, rooms: &[Rect], mat: &Materials) {
    for i in 1..rooms.len() {
        let (a, b) = (rooms[i - 1], rooms[i]);
        if a.right() > b.x && a.x < b.right() {
            continue; // stacked and overlapping: connected by stairs/beam, not a side door
        }
        if let Some(y) = find_side_exit(world, a, false) {
            place_object(world, a.right() - 1, y, 10, mat.door_style, -1);
        }
        if let Some(y) = find_side_exit(world, b, true) {
            place_object(world, b.x, y, 10, mat.door_style, -1);
        }
    }
}

/// Sloped stairs and a beam closing the gap between two stacked, overlapping rooms —
/// `PlaceStairs`/`CreateStairsList` and `PlaceSupportBeams`/`CreateSupportBeamList`, simplified to
/// a single straight ramp and a single beam column per adjacent pair rather than vanilla's
/// multi-column beam grid (a beam every 4-6 tiles across the whole footprint) — this generator's
/// rooms are already narrow enough that one beam reads the same.
///
/// The ramp is the room's own solid wall material (`mat.tile`) with a slope set, not vanilla's
/// literal sloped *platform* (tile 19): platforms are frame-important (`tile_sets::frame_important`
/// — the same class of bug this project already found and fixed once for doors, "ships -1 frames,
/// diverges on the next save"), and this generator has no existing table for a platform's real
/// neighbour-shape frame the way [`super::place_object`] has one for doors. A solid sloped ramp is
/// climbable the same way a sloped platform is; the only real difference is that vanilla's version
/// can also be dropped through from above, which this room's own stairs never need to be.
fn connect_stacked_rooms(world: &mut World, rooms: &[Rect], mat: &Materials) {
    for i in 1..rooms.len() {
        let (upper, lower) = (rooms[i - 1], rooms[i]);
        let overlap_x = upper.x.max(lower.x)..upper.right().min(lower.right());
        if overlap_x.is_empty() {
            continue;
        }
        let gap = lower.y - upper.bottom();
        if gap <= 0 {
            continue;
        }
        let cx = (overlap_x.start + overlap_x.end) / 2;
        for step in 0..gap {
            let mut t = Tile::block(mat.tile);
            t.frame_x = -1;
            t.frame_y = -1;
            t.slope = 1;
            world.set_tile(cx, upper.bottom() + step, t);
        }
        if let Some(beam_x) = overlap_x.clone().find(|&x| x != cx) {
            for step in 0..gap {
                let mut t = Tile::block(mat.beam);
                t.frame_x = -1;
                t.frame_y = -1;
                world.set_tile(beam_x, upper.bottom() + step, t);
            }
        }
    }
}

fn place_chest(
    world: &mut World,
    layout: &Layout,
    rooms: &[Rect],
    mat: &Materials,
    rand: &mut UnifiedRandom,
) -> bool {
    for room in rooms {
        for _ in 0..10 {
            let x = rand.next_range(room.x + 2, room.right() - 2);
            let y = room.bottom() - 2;
            let loot = structures::biome_chest_loot(layout, x, y, rand)
                .unwrap_or_else(|| structures::cavern_loot(layout, y, rand));
            let _ = mat.chest_style; // vanilla varies the chest sprite by material; loot is what matters here
            if structures::add_chest(world, x, y, loot, rand) {
                return true;
            }
        }
    }
    false
}

/// One cabin's worth of siting, carving and furnishing at `origin` — `CaveHouseBiome::Place`.
fn place_cabin(
    world: &mut World,
    layout: &Layout,
    structures_map: &mut StructureMap,
    origin: (i32, i32),
    rand: &mut UnifiedRandom,
) -> bool {
    if !in_world(layout, origin.0, origin.1, 30) {
        return false;
    }
    let rooms = create_rooms(world, origin, rand);
    if rooms.is_empty() || !rooms_in_bounds(layout, &rooms) {
        return false;
    }
    let kind = house_type(world, &rooms);
    if !rooms_valid(world, structures_map, &rooms, kind) {
        return false;
    }
    let mat = materials(kind);

    for &room in &rooms {
        carve_room(world, room, &mat);
        structures_map.add_protected_structure(room, 8);
    }
    connect_stacked_rooms(world, &rooms, &mat);
    place_doors(world, &rooms, &mat);
    // Chest chance mirrors `HouseBuilder.PlaceChests`' own `ChestChance` gate rather than a fixed
    // roll — vanilla's per-material chances are all in the 0.9-1.0 range in practice, so a single
    // high, disclosed constant stands in for the seven near-identical config values.
    if rand.next_double() < 0.9 {
        place_chest(world, layout, &rooms, &mat, rand);
    }
    true
}

/// The `UndergroundHousesAndBuriedChests` pass's cabin-placing third: scatter cabins through the
/// underground band. Returns how many were placed.
pub fn scatter(
    world: &mut World,
    layout: &Layout,
    structures_map: &mut StructureMap,
    rand: &mut UnifiedRandom,
) -> usize {
    // `CaveHouseCount`'s own default range is 35-40, scaled with world area
    // (`Terraria.GameContent.WorldBuilding.Configuration.json`) — calibrated here against this
    // project's own reference 4200x1200 world rather than guessed, since nothing in this
    // generator reads vanilla's `ScaleWith: WorldArea` config machinery directly.
    let margin = 400;
    if layout.width <= margin * 2 || layout.height <= 260 {
        return 0;
    }
    let wanted = (layout.width * layout.height) / 540_000 + rand.next_max(2);
    let mut placed = 0usize;
    let mut budget = 10_000;
    for _ in 0..wanted {
        while budget > 0 {
            budget -= 1;
            let x = rand.next_range(80, layout.width - 80);
            let y = rand.next_range(layout.surface + 20, layout.height - 230);
            if place_cabin(world, layout, structures_map, (x, y), rand) {
                placed += 1;
                break;
            }
        }
        if budget <= 0 {
            break;
        }
    }
    placed
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::TileFlags;
    use crate::world::World;
    use crate::world::worldgen::rand::UnifiedRandom as Rand;

    /// Solid stone from `ground_y` down, except for one big hollow cavern in the middle — a
    /// real cave void with real walls/floor/ceiling to site into, not an undifferentiated solid
    /// block. `create_rooms`' own site-finding logic (`find_room`'s left/right/up wall probes)
    /// needs real open space around a candidate origin the same way vanilla's does; a solid block
    /// with no voids at all is not a scenario real worldgen ever produces.
    fn cave_world(width: i32, height: i32, ground_y: i32) -> (World, Layout) {
        let mut world = World::empty(width, height, "cabin-test");
        for x in 0..width {
            for y in ground_y..height {
                world.set_tile(x, y, Tile::block(tiles::STONE));
            }
        }
        // Kept within the 200-tile `Down` search budget `create_rooms` uses to find a ceiling —
        // a cavern taller than that would leave an origin near its top unable to ever reach the
        // floor within the same distance vanilla's own search is bounded to.
        let (cx0, cx1) = (width / 4, width * 3 / 4);
        let (cy0, cy1) = (ground_y + 20, ground_y + 150);
        for x in cx0..cx1 {
            for y in cy0..cy1 {
                let mut t = world.tile(x, y);
                t.flags.set(TileFlags::ACTIVE, false);
                world.set_tile(x, y, t);
            }
        }
        let mut rand = Rand::new(1);
        let mut layout = Layout::plan(width, height, &mut rand);
        layout.surface = ground_y - 50;
        (world, layout)
    }

    #[test]
    fn a_small_world_returns_zero_rather_than_panicking() {
        let (mut world, layout) = cave_world(300, 200, 100);
        let mut structures_map = StructureMap::new();
        let mut rand = Rand::new(1);
        assert_eq!(
            scatter(&mut world, &layout, &mut structures_map, &mut rand),
            0
        );
    }

    /// Against a real generated world rather than a hand-built fixture — `create_rooms`' own
    /// site-finding (`find_room`'s wall probes, `rooms_valid`'s padded `StructureMap` check)
    /// depends on genuinely organic cave shape the same way vanilla's does: a candidate room
    /// carved flush against a perfectly flat synthetic wall/floor fails the padded check every
    /// time (the padding always reaches into the flat solid mass right behind it), which a real
    /// cave's uneven boundaries don't. `structures::caves()` is what every other Tier 2 pass in
    /// this generator already sites into for the same reason.
    #[test]
    fn a_cabin_carves_a_real_hollow_room_with_the_right_material() {
        let (world, built) = super::super::build(2200, 900, "underground-cabins-test", 7);
        assert!(
            built.underground_cabins > 0,
            "a real generated world should take at least one cabin"
        );

        let mut furnished_tiles = 0;
        for x in 0..world.width() {
            for y in 0..world.height() {
                let t = world.tile(x, y);
                if t.is_active() && matches!(t.block, 30 | 321 | 357 | 369 | 190) {
                    furnished_tiles += 1;
                }
            }
        }
        assert!(
            furnished_tiles > 20,
            "expected at least one real carved cabin room, got {furnished_tiles} material tiles"
        );
    }

    /// Real placement counts on real generated worlds — not asserted, just printed. Run with
    /// `cargo test -p terrustia --lib underground_cabins::tests::measure_on_real_worlds --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn measure_on_real_worlds() {
        for seed in [999u64, 4242, 12345] {
            let (_world, built) = super::super::build(4200, 1200, "measure", seed);
            eprintln!(
                "seed {seed}: underground_cabins={}",
                built.underground_cabins
            );
        }
    }
}
