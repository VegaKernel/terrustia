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
use super::rand::UnifiedRandom;
use super::tiles::{self, walls};
use crate::world::{Chest, World};

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
        let clear = (0..width).all(|dx| (0..height).all(|dy| !world.tile(x + dx, y - dy).is_active()));
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
            hollow_blob(world, x as i32, y as i32, radius, rand);
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
            let mut tile = Tile::framed(
                orb_tile,
                frame_x + (dx as i16) * 18,
                (dy as i16) * 18,
            );
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
        let x = rand.next_range(layout.jungle.from + 30, (layout.jungle.to - 30).max(layout.jungle.from + 31));
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
                let mut tile = Tile::framed(
                    tiles::LARVA,
                    (dx as i16) * 18,
                    (dy as i16) * 18,
                );
                tile.wall = walls::JUNGLE;
                world.set_tile(x + dx, floor + dy, tile);
            }
        }
        return true;
    }
    false
}

/// Chests, scattered through the caverns with tiered loot.
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
        if add_chest(world, x, feet, cavern_loot(layout, feet, rand), rand) {
            placed += 1;
        }
    }
    placed
}

/// Put a chest down if there is room and a floor for it.
fn add_chest(
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

/// What a cavern chest holds. Deeper is better, which is the whole of the reward curve.
fn cavern_loot(
    layout: &Layout,
    y: i32,
    rand: &mut UnifiedRandom,
) -> Vec<terrustia_proto::ItemStack> {
    use terrustia_proto::ItemStack;
    // The signature item, which is what a player opens a chest hoping for.
    let shallow = [
        49,   // Blue Phaseblade... a placeholder tier
        965,  // Shoe Spikes
        930,  // Cloud in a Bottle
        158,  // Hermes Boots
        963,  // Bandage
        997,  // Magic Mirror
    ];
    let deep = [
        119,  // Band of Regeneration
        155,  // Enchanted Boomerang
        1300, // Spelunker Potion... treated as a find
        997,  // Magic Mirror
        281,  // Aqua Scepter
        3068, // Extractinator-adjacent tier
    ];
    let pool: &[i32] = if y > layout.rock + 120 { &deep } else { &shallow };
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
