//! The things a world needs before it can be played through.
//!
//! Terrain alone is scenery. What makes a world *beatable* is a specific list, and every item on
//! it gates something:
//!
//! | Structure | Without it |
//! |---|---|
//! | Evil biome with orbs or hearts | no Eater of Worlds or Brain of Cthulhu, so no demonite, so no meteor |
//! | Dungeon | no Skeletron, and nothing behind him |
//! | Underworld with hellstone | no Wall of Flesh, so no hardmode |
//! | Demon altars | no hardmode ores, so nothing to fight the mechanical bosses with |
//! | Jungle temple | no Golem |
//! | Life crystals | a hundred hit points for the whole game |
//! | Chests | no starter weapons, no hooks, no boots |
//!
//! None of this is vanilla-identical and it does not try to be — see `docs/worldgen.md`. It is
//! built to be *complete and playable*, which is a different target and a reachable one.

use terrustia_proto::Tile;

use super::layout::{Evil, Layout, Surface};
use super::place_object::place_object;
use super::rand::UnifiedRandom;
use super::tiles::{self, walls};
use crate::world::{Chest, World};

/// The Lihzahrd Altar. Using a Lihzahrd Power Cell on it is the only way to fight Golem.
///
/// Not a decoration: a real client will not let a player attempt the use-item interaction at all
/// without an active tile of this type nearby, so a temple with none anywhere in the world made
/// Golem permanently unreachable through ordinary play in every world this generator has ever
/// produced — found by a worldgen sizing pass that happened to read vanilla's separate
/// `LihzahrdAltar` generation pass (`WorldGen.cs:22131`) and noticed `structures::temple` never
/// calls anything like it. `terrustia-proto`'s own `tile_object` table confirms the shape
/// independently: entry 237 is a 3-wide, 2-tall object with origin `(1, 1)` — the bottom-middle
/// cell, which is what `place_object`'s anchor argument expects.
const LIHZAHRD_ALTAR: u16 = 237;

/// Hollow out a tile, leaving its wall and liquid behind so a cave looks like a cave.
///
/// The frame has to go with the block. An inactive tile that still carries a frame is
/// inconsistent state that nothing notices until it is saved — the format writes no frame for an
/// inactive tile, so it reads back different from what was written, and a round-trip check that
/// should be exact comes back five tiles short. That is exactly how this was found.
fn hollow(world: &mut World, x: i32, y: i32) {
    if !world.in_bounds(x, y) {
        return;
    }
    let was = world.tile(x, y);
    let mut tile = Tile::AIR;
    tile.wall = was.wall;
    tile.wall_color = was.wall_color;
    tile.liquid = was.liquid;
    tile.liquid_kind = was.liquid_kind;
    world.set_tile(x, y, tile);
}

/// Fill a tile with something, keeping whatever wall was there.
fn place(world: &mut World, x: i32, y: i32, block: u16) {
    if !world.in_bounds(x, y) {
        return;
    }
    let wall = world.tile(x, y).wall;
    let mut tile = Tile::block(block);
    tile.wall = wall;
    world.set_tile(x, y, tile);
}

/// Fill a tile *and* its wall.
fn place_with_wall(world: &mut World, x: i32, y: i32, block: u16, wall: u16) {
    if !world.in_bounds(x, y) {
        return;
    }
    let mut tile = Tile::block(block);
    tile.wall = wall;
    world.set_tile(x, y, tile);
}

/// Find a place to stand something of a given width, by falling until there is a floor.
///
/// Picking a random point and hoping it lands on a ledge almost never works — most of a world is
/// either solid or open air, and a ledge is the thin boundary between them. The first version of
/// the altar pass did exactly that and put down one altar where it wanted twelve. Falling from a
/// random point instead finds the floor beneath it, which is what a player would do.
///
/// Returns the row the thing's *feet* go on, with `height` rows of clear air above it.
fn find_ledge(
    world: &World,
    x: i32,
    from_y: i32,
    to_y: i32,
    width: i32,
    height: i32,
) -> Option<i32> {
    let mut y = from_y;
    while y < to_y {
        let floored = (0..width).all(|dx| world.tile(x + dx, y + 1).is_active());
        let clear =
            (0..width).all(|dx| (0..height).all(|dy| !world.tile(x + dx, y - dy).is_active()));
        if floored && clear {
            return Some(y);
        }
        y += 1;
    }
    None
}

/// A rough disc of hollow, which is the shape almost everything here is made of.
fn hollow_blob(world: &mut World, cx: i32, cy: i32, radius: i32, rand: &mut UnifiedRandom) {
    let wobble = rand.next_range(-1, 2);
    for x in cx - radius - 1..=cx + radius + 1 {
        for y in cy - radius - 1..=cy + radius + 1 {
            let (dx, dy) = (x - cx, y - cy);
            if dx * dx + dy * dy <= (radius + wobble) * (radius + wobble) {
                hollow(world, x, y);
            }
        }
    }
}

/// Hollow out a tile the way [`hollow`] does, but clear its wall too.
///
/// `caves()` is the one carver that needs this. Every other structure here keeps `hollow`'s wall
/// as-is on purpose — a dungeon room or a chasm reads as a built space, not a hole into the void,
/// precisely because something is still behind it. Vanilla's own wandering cave tunnels are
/// different: `WorldGen.cs` registers `CaveWallsInEnclosedSpaces`/`CaveWallVariety` (the passes
/// that put wall behind a cave) as **Tier 3** work, run only after `GemCaves`/`SpiderCaves`/
/// `LivingTrees` (Tier 2) have already sited into the unwalled space those earlier passes carved —
/// so at the point in vanilla's own pipeline that matters here, a freshly-dug cave tunnel has no
/// wall at all. `caves()` carrying wall forward from the terrain pass that ran before it skipped
/// straight past that intermediate state, and it is exactly the state `cave_flood::count`'s own
/// wall check (transcribed from vanilla's `nextCount`) is built to search for: measured on a real
/// generated world, every sampled pocket saturated to the search cap before this fix, because
/// there was no unwalled pocket anywhere to find.
fn hollow_no_wall(world: &mut World, x: i32, y: i32) {
    if !world.in_bounds(x, y) {
        return;
    }
    let was = world.tile(x, y);
    let mut tile = Tile::AIR;
    tile.liquid = was.liquid;
    tile.liquid_kind = was.liquid_kind;
    world.set_tile(x, y, tile);
}

/// [`hollow_blob`], but through [`hollow_no_wall`] — see its doc comment for why `caves()` alone
/// needs this and nothing else here does.
fn hollow_blob_no_wall(world: &mut World, cx: i32, cy: i32, radius: i32, rand: &mut UnifiedRandom) {
    let wobble = rand.next_range(-1, 2);
    for x in cx - radius - 1..=cx + radius + 1 {
        for y in cy - radius - 1..=cy + radius + 1 {
            let (dx, dy) = (x - cx, y - cy);
            if dx * dx + dy * dy <= (radius + wobble) * (radius + wobble) {
                hollow_no_wall(world, x, y);
            }
        }
    }
}

/// ...and the same in a material.
fn fill_blob(world: &mut World, cx: i32, cy: i32, radius: i32, block: u16) {
    for x in cx - radius..=cx + radius {
        for y in cy - radius..=cy + radius {
            let (dx, dy) = (x - cx, y - cy);
            if dx * dx + dy * dy <= radius * radius {
                place(world, x, y, block);
            }
        }
    }
}

/// Wandering tunnels through the stone.
///
/// Walked rather than drawn: a tunnel that turns a little each step and widens and narrows as it
/// goes reads as a cave, where anything drawn from a formula reads as a corridor.
pub fn caves(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) {
    let count = (layout.width / 22).max(20);
    for _ in 0..count {
        let mut x = f64::from(rand.next_range(20, layout.width - 20));
        let mut y = f64::from(rand.next_range(layout.rock - 30, layout.underworld - 40));
        let mut angle = f64::from(rand.next_range(0, 628)) / 100.0;
        let length = rand.next_range(60, 420);
        let mut radius = rand.next_range(2, 5);

        for _ in 0..length {
            angle += f64::from(rand.next_range(-30, 31)) / 100.0;
            // Caves trend sideways rather than straight down, which is what makes them
            // walkable rather than a set of shafts.
            let step = angle.sin() * 0.55;
            x += angle.cos() * 1.4;
            y += step;
            if rand.next_max(40) == 0 {
                radius = (radius + rand.next_range(-1, 2)).clamp(2, 7);
            }
            if x < 8.0 || y < 8.0 || x > f64::from(layout.width - 8) {
                break;
            }
            if y > f64::from(layout.underworld - 10) {
                break;
            }
            hollow_blob_no_wall(world, x as i32, y as i32, radius, rand);
        }
    }
}

/// Ore, in the depth bands the game puts each metal in.
///
/// The bands are what make progression work: copper and iron near the surface so a new character
/// can find a pickaxe's worth, gold and silver deeper so they are worth going down for.
pub fn ores(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) {
    // (ore, from, to, how many per thousand columns, vein size)
    let bands: [(u16, i32, i32, i32, i32); 4] = [
        (tiles::COPPER, layout.surface, layout.underworld, 34, 5),
        (tiles::IRON, layout.surface + 20, layout.underworld, 28, 5),
        (tiles::SILVER, layout.rock, layout.underworld, 20, 4),
        (tiles::GOLD, layout.rock + 60, layout.underworld, 14, 4),
    ];
    for (ore, from, to, density, size) in bands {
        if to <= from + 4 {
            continue;
        }
        let veins = layout.width * density / 1000;
        for _ in 0..veins {
            let x = rand.next_range(10, layout.width - 10);
            let y = rand.next_range(from, to);
            // Only into stone: an ore vein hanging in a cave is not a vein.
            if world.tile(x, y).block != tiles::STONE {
                continue;
            }
            fill_blob(world, x, y, rand.next_range(1, size), ore);
        }
    }

    // Gems, which are rarer and only deep.
    let gems = [
        tiles::AMETHYST,
        tiles::TOPAZ,
        tiles::SAPPHIRE,
        tiles::EMERALD,
        tiles::RUBY,
        tiles::DIAMOND,
    ];
    for _ in 0..layout.width / 22 {
        let x = rand.next_range(10, layout.width - 10);
        let from = layout.rock + 100;
        let to = layout.underworld.max(from + 8);
        let y = rand.next_range(from, to);
        if world.tile(x, y).block != tiles::STONE {
            continue;
        }
        let gem = gems[rand.next_max(gems.len() as i32) as usize];
        fill_blob(world, x, y, rand.next_range(1, 3), gem);
    }
}

/// The evil biome's chasms, and the orbs or hearts at the bottom of them.
///
/// This is the one structure whose *contents* are progression rather than decoration: three orbs
/// smashed is the Eater of Worlds or the Brain of Cthulhu, and that is the whole of the first
/// act's gating.
pub fn evil_chasms(
    world: &mut World,
    layout: &Layout,
    heights: &[i32],
    rand: &mut UnifiedRandom,
) -> usize {
    let orb_tile = tiles::SHADOW_ORB;
    let chasms = 3 + rand.next_max(3);
    let mut orbs = 0;

    for nth in 0..chasms {
        // Spread the chasms across the band rather than stacking them.
        let band = layout.evil_band;
        let step = band.width() / (chasms + 1).max(1);
        let x = band.from + step * (nth + 1) + rand.next_range(-step / 3, step / 3 + 1);
        if x <= 2 || x >= layout.width - 2 {
            continue;
        }
        let top = heights[x.clamp(0, layout.width - 1) as usize];
        let bottom = (top + rand.next_range(90, 190)).min(layout.underworld - 40);

        // A chasm is a narrow shaft that widens as it goes down.
        let mut cx = x;
        for y in top..bottom {
            let along = f64::from(y - top) / f64::from((bottom - top).max(1));
            let half = (2.0 + along * 5.0) as i32;
            for dx in -half..=half {
                hollow(world, cx + dx, y);
            }
            // The shaft wanders, so it is not a drilled hole.
            if rand.next_max(7) == 0 {
                cx += rand.next_range(-1, 2);
            }
        }

        // A pocket at the bottom, with an orb in it.
        hollow_blob(world, cx, bottom, 6, rand);
        let orb_y = bottom + 2;
        // Frames say which half of the sheet the sprite comes from, and a crimson heart is the
        // right-hand half — `frameX >= 36`, which is what the break handler reads to decide
        // which boss to wake. Getting it wrong gives a corruption world crimson hearts.
        let frame_x: i16 = if layout.evil == Evil::Crimson { 36 } else { 0 };
        for (dx, dy) in [(0i32, 0i32), (1, 0), (0, 1), (1, 1)] {
            let mut tile = Tile::framed(orb_tile, frame_x + (dx as i16) * 18, (dy as i16) * 18);
            tile.wall = walls::EBONSTONE;
            world.set_tile(cx + dx, orb_y + dy, tile);
        }
        orbs += 1;
    }
    orbs
}

/// The demon altars: where hardmode ore comes from.
///
/// Scattered through the evil biome and the caverns. Without them a world stops dead at the Wall
/// of Flesh, because smashing them is the only source of cobalt, mythril and adamantite.
pub fn altars(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) -> usize {
    let wanted = (layout.width / 120).max(12) as usize;
    let mut placed = 0usize;
    let frame_base: i16 = if layout.evil == Evil::Crimson { 54 } else { 0 };

    for _ in 0..wanted * 40 {
        if placed >= wanted {
            break;
        }
        // Two thirds in the evil band, the rest anywhere underground — which is where a player
        // who has cleared their own biome goes looking for the last few.
        let x = if rand.next_max(3) > 0 && layout.evil_band.width() > 8 {
            rand.next_range(layout.evil_band.from + 3, layout.evil_band.to - 3)
        } else {
            rand.next_range(20, layout.width - 20)
        };
        let from = rand.next_range(layout.surface + 30, layout.underworld - 40);
        // An altar is three wide and two tall, and needs a floor under all three.
        let Some(y) = find_ledge(world, x, from, layout.underworld - 20, 3, 2) else {
            continue;
        };

        for dx in 0..3i32 {
            for dy in 0..2i32 {
                let wall = world.tile(x + dx, y - 1 + dy).wall;
                let mut tile = Tile::framed(
                    tiles::DEMON_ALTAR,
                    frame_base + (dx as i16) * 18,
                    (dy as i16) * 18,
                );
                tile.wall = wall;
                world.set_tile(x + dx, y - 1 + dy, tile);
            }
        }
        placed += 1;
    }
    placed
}

/// Life crystals, which are the only way past a hundred hit points.
pub fn life_crystals(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) -> usize {
    let wanted = (layout.width / 90).max(15) as usize;
    let mut placed = 0usize;
    for _ in 0..wanted * 60 {
        if placed >= wanted {
            break;
        }
        let x = rand.next_range(20, layout.width - 20);
        let from = rand.next_range(layout.rock, layout.underworld - 40);
        // Two wide and two tall, standing on something.
        let Some(feet) = find_ledge(world, x, from, layout.underworld - 20, 2, 2) else {
            continue;
        };
        for dx in 0..2i32 {
            for dy in 0..2i32 {
                let y = feet - 1 + dy;
                let wall = world.tile(x + dx, y).wall;
                let mut tile = Tile::framed(tiles::HEART, (dx as i16) * 18, (dy as i16) * 18);
                tile.wall = wall;
                world.set_tile(x + dx, y, tile);
            }
        }
        placed += 1;
    }
    placed
}

/// The dungeon: a warren of brick rooms behind the door Skeletron guards.
///
/// Deliberately simple compared with the game's — rooms on a grid joined by corridors — because
/// what the dungeon has to *be* for a playthrough is a large walled space full of dungeon brick,
/// with an entrance at the surface and chests inside. Its shape is atmosphere; its existence is
/// progression.
pub fn dungeon(world: &mut World, layout: &Layout, heights: &[i32], rand: &mut UnifiedRandom) {
    let brick = match rand.next_max(3) {
        0 => tiles::BLUE_DUNGEON_BRICK,
        1 => tiles::GREEN_DUNGEON_BRICK,
        _ => tiles::PINK_DUNGEON_BRICK,
    };
    let wall = match brick {
        tiles::BLUE_DUNGEON_BRICK => walls::BLUE_DUNGEON,
        tiles::GREEN_DUNGEON_BRICK => walls::GREEN_DUNGEON,
        _ => walls::PINK_DUNGEON,
    };

    let x = layout.dungeon_x.clamp(80, layout.width - 80);
    let entrance_y = heights[x as usize];
    let bottom = (layout.rock + 260).min(layout.underworld - 60);

    // A shaft from the surface down to the rooms, so the dungeon is reachable on foot.
    for y in entrance_y..bottom {
        for dx in -6..=6i32 {
            let edge = dx.abs() > 4;
            if edge {
                place_with_wall(world, x + dx, y, brick, wall);
            } else {
                place_with_wall(world, x + dx, y, 0, wall);
                hollow(world, x + dx, y);
            }
        }
    }

    // Rooms, spread either side of the shaft and down.
    let rooms = 14 + rand.next_max(10);
    for _ in 0..rooms {
        let rw = rand.next_range(14, 30);
        let rh = rand.next_range(9, 16);
        let rx = x + rand.next_range(-90, 91);
        let ry = rand.next_range(entrance_y + 30, bottom);
        if rx - rw < 10 || rx + rw > layout.width - 10 {
            continue;
        }

        for cx in rx - rw..=rx + rw {
            for cy in ry - rh..=ry + rh {
                let edge = cx == rx - rw || cx == rx + rw || cy == ry - rh || cy == ry + rh;
                if edge {
                    place_with_wall(world, cx, cy, brick, wall);
                } else {
                    place_with_wall(world, cx, cy, 0, wall);
                    hollow(world, cx, cy);
                }
            }
        }

        // A corridor back to the shaft, so no room is sealed off.
        let corridor_y = ry;
        let (from, to) = if rx < x { (rx, x) } else { (x, rx) };
        for cx in from..=to {
            for cy in corridor_y - 2..=corridor_y + 2 {
                place_with_wall(world, cx, cy, 0, wall);
                hollow(world, cx, cy);
            }
        }

        // A chest in about half of them, with something worth the walk.
        if rand.next_max(2) == 0 {
            let cx = rx + rand.next_range(-rw + 2, rw - 2);
            let cy = ry + rh - 1;
            add_chest(world, cx, cy, dungeon_loot(rand), rand);
        }
    }
}

/// The jungle temple: lihzahrd brick, and nothing gets in until Plantera falls.
pub fn temple(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) {
    let (tx, ty) = layout.temple;
    let half_w = rand.next_range(34, 55);
    let half_h = rand.next_range(20, 32);

    for x in tx - half_w..=tx + half_w {
        for y in ty - half_h..=ty + half_h {
            // A thick shell, since the point of the temple is that it cannot be dug into.
            let edge = x < tx - half_w + 3
                || x > tx + half_w - 3
                || y < ty - half_h + 3
                || y > ty + half_h - 3;
            if edge {
                place_with_wall(world, x, y, tiles::LIHZAHRD_BRICK, walls::LIHZAHRD_BRICK);
            } else {
                place_with_wall(world, x, y, 0, walls::LIHZAHRD_BRICK);
                hollow(world, x, y);
            }
        }
    }

    // Inner walls, so it is a temple rather than a box.
    let rooms = rand.next_range(3, 6);
    for nth in 1..=rooms {
        let at = tx - half_w + (half_w * 2 / (rooms + 1)) * nth;
        for y in ty - half_h + 3..=ty + half_h - 3 {
            // A gap in each, so every room is reachable.
            if (y - (ty + half_h - 6)).abs() > 3 {
                place_with_wall(world, at, y, tiles::LIHZAHRD_BRICK, walls::LIHZAHRD_BRICK);
            }
        }
    }

    // The altar. It stands on the temple's own floor — the last hollow row before the shell's
    // bottom edge — so `place_object`'s footprint-and-floor check passes against the brick the
    // edge loop above already laid down. That row sits inside every inner room's doorway gap
    // (`(y - (ty+half_h-6)).abs() <= 3`, the same test the wall loop above uses to *skip* a wall),
    // so no inner wall can be standing where the altar needs to go, in any room it might land in.
    //
    // Centred first, since the interior is symmetric and centre is clear of both the outer shell
    // and every inner wall column by construction; a few fallback offsets cover the rare case
    // where a small `half_w` roll puts the centre awkwardly close to a wall column.
    let altar_y = ty + half_h - 3;
    let mut altar_placed = false;
    for dx in [0, -4, 4, -8, 8, -12, 12] {
        if place_object(world, tx + dx, altar_y, LIHZAHRD_ALTAR, 0, -1) {
            altar_placed = true;
            break;
        }
    }
    debug_assert!(
        altar_placed,
        "the jungle temple must always get an altar, or Golem is unreachable in this world"
    );
}

/// Hellstone and lava, which are what the underworld is for.
pub fn underworld(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) {
    // The band has to be worked out rather than assumed: a short world can leave less room under
    // the underworld's top than the constants want, and the generator throws on a backwards range
    // rather than quietly returning nonsense.
    let top = layout.underworld + 4;
    let floor = (layout.height - 12).max(top + 8);
    // Open it out: the underworld is a cavern, not solid ash.
    for _ in 0..layout.width / 4 {
        let x = rand.next_range(10, layout.width - 10);
        let y = rand.next_range(top, floor);
        hollow_blob(world, x, y, rand.next_range(4, 12), rand);
    }
    // A floor of lava across most of the bottom, which is what makes crossing it a problem.
    let lava_line = ((layout.height - 32).max(top + 4)).min(layout.height - 4);
    for x in 0..layout.width {
        for y in lava_line..(layout.height - 2) {
            let mut tile = world.tile(x, y);
            if !tile.is_active() {
                tile.liquid = 255;
                tile.liquid_kind = terrustia_proto::Liquid::Lava;
                world.set_tile(x, y, tile);
            }
        }
    }
    // Hellstone, which is the only thing down here worth the trip.
    for _ in 0..layout.width / 6 {
        let x = rand.next_range(10, layout.width - 10);
        let y = rand.next_range(top, floor);
        if world.tile(x, y).block != tiles::ASH {
            continue;
        }
        fill_blob(world, x, y, rand.next_range(2, 5), tiles::HELLSTONE);
    }
}

/// A bee hive in the jungle, with the larva that wakes the Queen.
pub fn hive(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) -> bool {
    for _ in 0..80 {
        let x = rand.next_range(
            layout.jungle.from + 30,
            (layout.jungle.to - 30).max(layout.jungle.from + 31),
        );
        let from = layout.rock + 40;
        let to = (layout.rock + 200)
            .min(layout.underworld - 40)
            .max(from + 8);
        let y = rand.next_range(from, to);
        if world.tile(x, y).block != tiles::MUD {
            continue;
        }
        let radius = rand.next_range(11, 18);
        fill_blob(world, x, y, radius, tiles::HIVE);
        hollow_blob(world, x, y, radius - 3, rand);
        // The larva, which is the only way to call the Queen without a summon item.
        let floor = y + radius - 4;
        for dx in 0..2i32 {
            for dy in 0..2i32 {
                let mut tile = Tile::framed(tiles::LARVA, (dx as i16) * 18, (dy as i16) * 18);
                tile.wall = walls::JUNGLE;
                world.set_tile(x + dx, floor + dy, tile);
            }
        }
        return true;
    }
    false
}

/// Chests, scattered through the caverns with tiered loot.
///
/// The signature item is vanilla's own where the biome and depth match one it treats specially —
/// jungle and underground desert, both transcribed from `AddBuriedChest`'s own item selection
/// (`WorldGen.cs:36429-36447` for the jungle roll, `:36404-36420` for the desert one). Everywhere
/// else keeps the existing depth-tiered table: vanilla's own selection there is not self-contained
/// in this function — the underworld's rotates through a shuffled array set up during world
/// generation elsewhere, and this generator does not model underground-desert as a region distinct
/// from a plain deep desert column, so a biome-tagged column just below `rock` is treated as one.
pub fn chests(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) -> usize {
    let wanted = (layout.width / 14).max(60) as usize;
    let mut placed = 0usize;
    for _ in 0..wanted * 30 {
        if placed >= wanted {
            break;
        }
        let x = rand.next_range(20, layout.width - 20);
        let from = rand.next_range(layout.surface + 10, layout.underworld - 40);
        let Some(feet) = find_ledge(world, x, from, layout.underworld - 20, 2, 2) else {
            continue;
        };
        let loot = biome_chest_loot(layout, x, feet, rand)
            .unwrap_or_else(|| cavern_loot(layout, feet, rand));
        if add_chest(world, x, feet, loot, rand) {
            placed += 1;
        }
    }
    placed
}

/// Put a chest down if there is room and a floor for it.
///
/// `pub(crate)`: `jungle_shrines.rs` reuses this directly for the chest a shrine's own floor gap
/// is built to hold, rather than re-deriving the same clearance/floor check a second time.
pub(crate) fn add_chest(
    world: &mut World,
    x: i32,
    y: i32,
    items: Vec<terrustia_proto::ItemStack>,
    _rand: &mut UnifiedRandom,
) -> bool {
    if !world.in_bounds(x, y) || !world.in_bounds(x + 1, y + 1) {
        return false;
    }
    // Two by two of air with solid ground beneath.
    let clear = (0..2).all(|dx| (0..2).all(|dy| !world.tile(x + dx, y - dy).is_active()));
    let floored = (0..2).all(|dx| world.tile(x + dx, y + 1).is_active());
    if !clear || !floored {
        return false;
    }
    if world.chest_at(x as i16, (y - 1) as i16).is_some() {
        return false;
    }

    for dx in 0..2i32 {
        for dy in 0..2i32 {
            let wall = world.tile(x + dx, y - 1 + dy).wall;
            let mut tile = Tile::framed(tiles::CHEST, (dx as i16) * 18, (dy as i16) * 18);
            tile.wall = wall;
            world.set_tile(x + dx, y - 1 + dy, tile);
        }
    }
    let mut chest = Chest::empty_at(x as i16, (y - 1) as i16);
    for (slot, item) in items.into_iter().enumerate() {
        if let Some(cell) = chest.items.get_mut(slot) {
            *cell = item;
        }
    }
    world.add_chest(chest);
    true
}

/// Vanilla's real jungle-chest and underground-desert-chest signature items, if this site is
/// biome-tagged for one of them. `None` for everywhere else, so the caller falls back to the
/// existing depth-tiered table.
///
/// Transcribed from `AddBuriedChest`, which does not pick these by a clean per-style switch —
/// it derives them from a chain of boolean flags gated on the chest's site. The two item lists
/// below are exactly its `flag2` (jungle) and the desert-tool block near the top of the function,
/// item IDs and roll odds unchanged.
///
/// `pub(crate)`: `jungle_shrines.rs` reuses the jungle branch for its own chest, in place of
/// vanilla's separate `GetNextJungleChestItem` (a non-repeating cycle through the same signature
/// list) — a real, disclosed simplification: both draw from the same underlying jungle item set,
/// and duplicating a second selection mechanism for that difference alone was not worth it.
pub(crate) fn biome_chest_loot(
    layout: &Layout,
    x: i32,
    y: i32,
    rand: &mut UnifiedRandom,
) -> Option<Vec<terrustia_proto::ItemStack>> {
    use terrustia_proto::ItemStack;

    if layout.jungle.contains(x) {
        // WorldGen.cs:36429 `num10 = Utils.SelectRandom(genRand, new short[7] { 670, 724, 950,
        // 1319, 987, 1579, 6153 })`, then a further one-in-twenty reroll to 997 at :36444.
        const JUNGLE: [i32; 7] = [670, 724, 950, 1319, 987, 1579, 6153];
        let mut signature = JUNGLE[rand.next_max(JUNGLE.len() as i32) as usize];
        if rand.next_max(20) == 0 {
            signature = 997;
        }
        let mut items = vec![ItemStack::new(signature, 1, 0)];
        items.push(ItemStack::new(8, rand.next_range(10, 30) as i16, 0));
        items.push(ItemStack::new(71, rand.next_range(10, 99) as i16, 0));
        return Some(items);
    }

    // Vanilla's underground desert is a region distinct from a plain desert column at depth —
    // sized against `GenVars.UndergroundDesertLocation`, which this generator does not carve as
    // its own shape. A desert-biome column once it is below the rock layer is treated as close
    // enough: it is what the surface desert becomes once you dig, which is the case this table
    // exists for.
    if layout.desert.contains(x) && y > layout.rock {
        // WorldGen.cs:36404 `num10 = Utils.SelectRandom(genRand, new short[4] { 4056, 4055, 4262,
        // 4263 })`. Vanilla has a second, rarer four-item set for the shallow half of the desert
        // hive band specifically; that band is not modelled here, so only the common set is used.
        const DESERT: [i32; 4] = [4056, 4055, 4262, 4263];
        let signature = DESERT[rand.next_max(DESERT.len() as i32) as usize];
        let mut items = vec![ItemStack::new(signature, 1, 0)];
        items.push(ItemStack::new(8, rand.next_range(10, 30) as i16, 0));
        items.push(ItemStack::new(71, rand.next_range(10, 99) as i16, 0));
        return Some(items);
    }

    None
}

/// What a cavern chest holds. Deeper is better, which is the whole of the reward curve.
///
/// `pub(crate)`: `underground_cabins.rs` reuses this directly for the one chest each cabin holds,
/// rather than re-deriving the same depth-tiered table a second time.
pub(crate) fn cavern_loot(
    layout: &Layout,
    y: i32,
    rand: &mut UnifiedRandom,
) -> Vec<terrustia_proto::ItemStack> {
    use terrustia_proto::ItemStack;
    // The signature item, which is what a player opens a chest hoping for.
    let shallow = [
        49,  // Blue Phaseblade... a placeholder tier
        965, // Shoe Spikes
        930, // Cloud in a Bottle
        158, // Hermes Boots
        963, // Bandage
        997, // Magic Mirror
    ];
    let deep = [
        119,  // Band of Regeneration
        155,  // Enchanted Boomerang
        1300, // Spelunker Potion... treated as a find
        997,  // Magic Mirror
        281,  // Aqua Scepter
        3068, // Extractinator-adjacent tier
    ];
    let pool: &[i32] = if y > layout.rock + 120 {
        &deep
    } else {
        &shallow
    };
    let signature = pool[rand.next_max(pool.len() as i32) as usize];

    let mut items = vec![ItemStack::new(signature, 1, 0)];
    // Torches and a rope are what actually make a cave chest useful.
    items.push(ItemStack::new(8, rand.next_range(10, 30) as i16, 0));
    items.push(ItemStack::new(965, rand.next_range(20, 60) as i16, 0));
    // A little money.
    items.push(ItemStack::new(71, rand.next_range(10, 99) as i16, 0));
    items
}

/// ...and what a dungeon chest holds, which is a tier above.
fn dungeon_loot(rand: &mut UnifiedRandom) -> Vec<terrustia_proto::ItemStack> {
    use terrustia_proto::ItemStack;
    let signature = [
        327,  // Muramasa
        328,  // Cobalt Shield
        329,  // Aqua Scepter
        330,  // Blue Moon
        676,  // Shadow Key adjacent
        1266, // Handgun tier
    ];
    let pick = signature[rand.next_max(signature.len() as i32) as usize];
    vec![
        ItemStack::new(pick, 1, 0),
        ItemStack::new(8, rand.next_range(15, 40) as i16, 0),
        ItemStack::new(72, rand.next_range(5, 40) as i16, 0),
    ]
}

/// Grass, and the surface plants that grow on it.
pub fn greenery(world: &mut World, layout: &Layout, heights: &[i32], rand: &mut UnifiedRandom) {
    for x in 0..layout.width {
        let top = heights[x as usize];
        if layout.surface_biome(x) == Some(Surface::Ocean) {
            continue;
        }
        let ground = world.tile(x, top).block;
        let plant = match ground {
            tiles::GRASS => tiles::PLANTS,
            _ => continue,
        };
        if rand.next_max(3) != 0 {
            continue;
        }
        if world.tile(x, top - 1).is_active() {
            continue;
        }
        let wall = world.tile(x, top - 1).wall;
        let mut tile = Tile::framed(plant, (rand.next_max(6) * 18) as i16, 0);
        tile.wall = wall;
        world.set_tile(x, top - 1, tile);
    }
}

/// Cobwebs, which is what tells a player a cave has not been visited.
pub fn cobwebs(world: &mut World, layout: &Layout, rand: &mut UnifiedRandom) {
    for _ in 0..layout.width * 2 {
        let x = rand.next_range(10, layout.width - 10);
        let y = rand.next_range(layout.rock, layout.underworld - 20);
        if world.tile(x, y).is_active() {
            continue;
        }
        // Only where something is adjacent to hang from.
        let anchored = [(0, -1), (0, 1), (-1, 0), (1, 0)]
            .iter()
            .any(|(dx, dy)| world.tile(x + dx, y + dy).is_active());
        if !anchored {
            continue;
        }
        let mut spread = rand.next_range(3, 14);
        let (mut cx, mut cy) = (x, y);
        while spread > 0 {
            spread -= 1;
            if !world.tile(cx, cy).is_active() {
                place(world, cx, cy, tiles::COBWEB);
            }
            match rand.next_max(4) {
                0 => cx += 1,
                1 => cx -= 1,
                2 => cy += 1,
                _ => cy -= 1,
            }
            if !world.in_bounds(cx, cy) {
                break;
            }
        }
    }
}

#[cfg(test)]
mod chest_loot_tests {
    use super::*;
    use crate::world::worldgen::layout::Band;

    /// A biome-tagged chest carries vanilla's real signature item, not the generic cavern table.
    ///
    /// Transcribed from `AddBuriedChest`'s own item lists (`WorldGen.cs:36429` jungle,
    /// `:36404` desert) rather than guessed — asserted against the exact vanilla item id set
    /// rather than merely "some item", since the whole point is that these are *vanilla's* ids.
    #[test]
    fn a_jungle_column_gets_vanillas_jungle_chest_items() {
        let mut layout = test_layout();
        layout.jungle = Band { from: 100, to: 200 };

        // Enough draws that both the main roll and the one-in-twenty 997 reroll are exercised.
        let mut seen_signature = false;
        let mut seen_reroll = false;
        for seed in 0..200i32 {
            let mut rand = UnifiedRandom::new(seed);
            let items = biome_chest_loot(&layout, 150, 500, &mut rand)
                .expect("a jungle-biome column must not fall through to the generic table");
            let signature = items[0].id;
            assert!(
                [670, 724, 950, 1319, 987, 1579, 6153, 997].contains(&signature),
                "unexpected jungle chest item id {signature}"
            );
            if signature != 997 {
                seen_signature = true;
            } else {
                seen_reroll = true;
            }
        }
        assert!(
            seen_signature,
            "the jungle table should produce its own items"
        );
        assert!(
            seen_reroll,
            "the one-in-twenty 997 reroll should show up over 200 draws"
        );
    }

    #[test]
    fn a_deep_desert_column_gets_vanillas_desert_chest_items() {
        let mut layout = test_layout();
        layout.desert = Band { from: 300, to: 400 };
        layout.rock = 400;

        for seed in 0..50i32 {
            let mut rand = UnifiedRandom::new(seed);
            let items = biome_chest_loot(&layout, 350, 500, &mut rand)
                .expect("a deep desert column must not fall through to the generic table");
            assert!(
                [4056, 4055, 4262, 4263].contains(&items[0].id),
                "unexpected desert chest item id {}",
                items[0].id
            );
        }
    }

    /// A shallow desert column — above the rock layer — is not vanilla's underground desert, and
    /// a non-biome column gets the ordinary depth-tiered table, not a biome one.
    #[test]
    fn everywhere_else_falls_back_to_the_generic_table() {
        let mut layout = test_layout();
        layout.jungle = Band { from: 100, to: 200 };
        layout.desert = Band { from: 300, to: 400 };
        layout.rock = 400;
        let mut rand = UnifiedRandom::new(1);

        // Shallow desert: above the rock layer.
        assert!(biome_chest_loot(&layout, 350, 100, &mut rand).is_none());
        // Plain caverns, in neither band.
        assert!(biome_chest_loot(&layout, 250, 500, &mut rand).is_none());
    }

    fn test_layout() -> Layout {
        let mut rand = UnifiedRandom::new(1);
        Layout::plan(2000, 800, &mut rand)
    }
}

/// Golem's fight is gated on a single tile that `temple()` used to never place.
///
/// A real client will not let a player attempt to use a Lihzahrd Power Cell at all without an
/// active `LIHZAHRD_ALTAR` tile nearby — this is a client-side gate on the interaction itself, not
/// something a server can work around after the fact. So a temple with no altar anywhere in the
/// world does not make Golem merely hard to reach; it makes Golem unreachable through any
/// legitimate play, in every world this generator has ever produced, forever, silently — the
/// worldgen module's own doc comment already claimed "Jungle temple → no Golem" as the reason the
/// temple exists at all, so this is a case where the code did not do what its own comment said it
/// did.
///
/// Run across several seeds and several of the random temple sizes `temple()` itself rolls
/// (`half_w` 34-55, `half_h` 20-32), rather than one lucky draw, because the fix's fallback offsets
/// exist specifically to cover sizes where the centre placement is not immediately clear.
#[cfg(test)]
mod temple_altar_tests {
    use super::*;

    #[test]
    fn every_temple_gets_a_lihzahrd_altar() {
        for seed in 0..40i32 {
            let mut rand = UnifiedRandom::new(seed);
            let mut world = World::empty(400, 300, "temple");
            let layout_rand = &mut UnifiedRandom::new(seed);
            let mut layout = Layout::plan(400, 300, layout_rand);
            layout.temple = (200, 150);

            temple(&mut world, &layout, &mut rand);

            let mut found = 0usize;
            for x in 140..260 {
                for y in 100..200 {
                    if world.tile(x, y).is_active() && world.tile(x, y).block == LIHZAHRD_ALTAR {
                        found += 1;
                    }
                }
            }
            assert!(
                found > 0,
                "seed {seed}: no Lihzahrd Altar tile anywhere in this temple — Golem would be \
                 unreachable in a world generated from this seed"
            );
        }
    }
}

#[cfg(test)]
mod cave_wall_tests {
    use super::*;

    /// `caves()` carves through solid, walled terrain. If it leaves the wall in place behind what
    /// it hollows out, every cave in a generated world ends up walled — so `cave_flood::count`
    /// (used to site `GemCaves`/`SpiderCaves`/`LivingTrees`, all of which reject any walled tile,
    /// matching vanilla's own `nextCount`) can never find a pocket to build in at all. Measured
    /// directly on a real generated world before this fix: every sampled deep-rock point saturated
    /// to the search cap.
    #[test]
    fn a_carved_cave_tile_has_no_wall() {
        let mut rand = UnifiedRandom::new(7);
        let mut layout_rand = UnifiedRandom::new(7);
        let mut world = World::empty(400, 300, "cave-wall");
        let layout = Layout::plan(400, 300, &mut layout_rand);

        // Solid, fully-walled ground everywhere first — the same state `terrain::fill` leaves
        // underground tiles in, which is exactly what `caves()` carves into during real
        // generation.
        for x in 0..400 {
            for y in 0..300 {
                let mut tile = Tile::block(tiles::STONE);
                tile.wall = walls::STONE;
                world.set_tile(x, y, tile);
            }
        }

        caves(&mut world, &layout, &mut rand);

        let mut carved = 0usize;
        let mut still_walled = 0usize;
        for x in 0..400 {
            for y in 0..300 {
                let tile = world.tile(x, y);
                if !tile.is_active() {
                    carved += 1;
                    if tile.wall != 0 {
                        still_walled += 1;
                    }
                }
            }
        }
        assert!(carved > 0, "caves() carved nothing on a fully solid world");
        assert_eq!(
            still_walled, 0,
            "{still_walled} of {carved} carved cave tiles still have a wall — a pass that sites \
             into unwalled pockets (gem caves, spider caves, living trees) can never find one"
        );
    }

    /// The same property, at real world size with a real seed rather than the small hand-built
    /// fixture above — this is the actual check that found the bug, not a stand-in for it.
    /// **Updated for Tier 3**: originally ran the full `generate()` pipeline and sampled its final
    /// output; now stops right after `terrain::fill` + `caves()`, the same two steps in the same
    /// order `build()` itself runs — see the test's own body comment for why the full pipeline is
    /// no longer the right thing to sample once `CaveWallVariety`/`CaveWallsInEnclosedSpaces`/
    /// `MossAndMossCaves` exist.
    ///
    /// **This fix alone does not unblock `GemCaves`/`SpiderCaves`'s siting, and this test does not
    /// claim it does — see the module-level note above `hollow_no_wall` and the caller's own
    /// report for the second, separate issue.** What this fix delivers, and what this test
    /// actually pins: real, open cave interiors are genuinely unwalled now, matching vanilla's own
    /// pre-`CaveWallsInEnclosedSpaces` pipeline state. Measured before this fix: nearly every open
    /// tile sampled from a real generated world still carried the wall painted by `terrain::fill`
    /// before caves were ever carved through it. That specific defect is what this asserts against.
    ///
    /// A *second*, independent defect was found while building this test and is not fixed here:
    /// even with walls correctly cleared, `cave_flood`-style pocket measurement from real sampled
    /// points still saturates to the 3500-tile search cap almost universally — terrustia's own
    /// cave carver (`caves()`, a wandering-tunnel algorithm, not vanilla's) produces caves that
    /// read as one large interconnected network rather than vanilla's mix of small isolated
    /// pockets and large caverns, the same shape of siting mismatch `lakes.rs`'s own doc comment
    /// already discloses for lake placement. Fixing *that* means reworking `caves()`'s own
    /// topology, a materially bigger and riskier change to already-shipped Tier 1 generation than
    /// this task's scope — flagged, not attempted.
    #[test]
    fn a_real_generated_world_has_real_unwalled_open_cave_tiles() {
        // Sampled right after `terrain::fill` + `caves()` — the same two steps `build()` itself
        // runs in this order — rather than the full `generate()` pipeline. Tier 3 landed since this
        // test was first written (`CaveWallVariety`/`CaveWallsInEnclosedSpaces`/`MossAndMossCaves`,
        // all wired in after `smooth()`, near the end of `build()`) legitimately re-walls a real
        // fraction of the open cave network by design — that is the whole point of those three
        // passes, not a regression of the fix this test guards. Sampling the *final* world would
        // now measure Tier 3 working correctly and mistake it for `caves()` regressing. What this
        // test actually guards — `caves()` itself leaving no wall behind on what it carves — is
        // only ever true immediately after `caves()` runs, before anything downstream paints a wall
        // back on; on the real, full pipeline that is a fact about *this specific ordering*, not
        // about the finished world.
        let seed = 4242;
        let (width, height) = (4200, 1200);
        let mut rand = UnifiedRandom::new(seed);
        let plan = Layout::plan(width, height, &mut rand);
        let mut world = World::empty(width, height, "cave-wall-real");
        world.crimson = plan.evil == crate::world::worldgen::layout::Evil::Crimson;
        let heights = crate::world::worldgen::terrain::heightmap(&plan, &mut rand);
        crate::world::worldgen::terrain::fill(&mut world, &plan, &heights, &mut rand);
        caves(&mut world, &plan, &mut rand);

        let (from, to) = (plan.rock + 30, world.height() - 200);
        let mut rng = 12345u64;
        let mut open_samples = 0u32;
        let mut walled_samples = 0u32;
        for _ in 0..300 {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let x = 20 + (((rng >> 16) as u32) % (world.width() as u32 - 40)) as i32;
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let y = from + (((rng >> 16) as u32) % (to - from).max(1) as u32) as i32;
            if world.tile(x, y).is_active() {
                continue;
            }
            open_samples += 1;
            if world.tile(x, y).wall != 0 {
                walled_samples += 1;
            }
        }
        assert!(
            open_samples > 20,
            "too few open samples ({open_samples}) to say anything about this world"
        );
        // Not zero: a blind sample across the whole deep-rock band also lands inside the
        // dungeon, the hive, the evil chasms and the jungle temple — every one of those still
        // uses `hollow`/`hollow_blob` and keeps its wall on purpose (see that function's own doc
        // comment), and this sampling has no way to tell "inside the dungeon" from "inside a
        // cave" without inspecting more than wall state. `caves()` carves far more of the deep
        // band than every other structure combined, so a large majority unwalled is the real
        // signal; a small minority walled is those other structures working as intended, not a
        // regression in this fix.
        let walled_fraction = f64::from(walled_samples) / f64::from(open_samples);
        assert!(
            walled_fraction < 0.2,
            "{walled_samples} of {open_samples} open points sampled from a real generated world \
             ({:.0}%) still carry a wall — too many to be only the dungeon/hive/chasms/temple; \
             caves() looks like it is leaving wall behind again",
            walled_fraction * 100.0
        );
    }
}
