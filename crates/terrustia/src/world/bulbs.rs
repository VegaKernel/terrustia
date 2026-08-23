//! Plantera's bulbs: the only way to reach her.
//!
//! Plantera has no summon item. Once all three mechanical bosses are down, bulbs start appearing
//! in the underground jungle, and breaking one is what wakes her — so without this the whole back
//! half of the game is unreachable.
//!
//! The search is the game's own: pick a point in a band on the jungle side of the world, walk
//! *up* the column until jungle grass is under you, and try to put a bulb on it. Two and a half
//! thousand tries, getting less fussy as they run out — after two thousand it will settle for mud
//! instead of grass, and after that it stops requiring the tile beneath to be solid at all. A
//! world where the jungle has been dug out still gets its bulb eventually.
//!
//! Transcribed from `WorldGen.GeneratePlanteraBulbOnAllMechsDefeated` in the 1.4.5.7 build.

use rand::Rng;
use rand::rngs::SmallRng;

use super::World;

/// The bulb tile, and the two the jungle is made of.
pub const BULB: u16 = 238;
pub const JUNGLE_GRASS: u16 = 60;
pub const MUD: u16 = 59;

/// How many points the search tries before giving up.
const ATTEMPTS: i32 = 2_500;
/// Below this many left, mud will do instead of jungle grass.
const RELAX_TO_MUD: i32 = 500;
/// ...and below this, the ground need not even be there.
const RELAX_TO_ANYTHING: i32 = 200;
/// How far up one column the walk will look.
const CLIMB: i32 = 500;

/// How deep below the world's bottom the underworld starts, which is where the jungle stops.
const UNDERWORLD: i32 = 200;

/// Try to grow one bulb somewhere in the underground jungle.
///
/// Returns where it went, or `None` if two and a half thousand tries found nowhere.
pub fn grow(world: &mut World, rng: &mut SmallRng) -> Option<(i32, i32)> {
    // The jungle is opposite the dungeon, so the band to search follows it.
    let dungeon_on_the_right = world.dungeon_x.is_some_and(|x| x > world.width() / 2);
    let (from_x, to_x) = if dungeon_on_the_right {
        (world.width() * 15 / 100, world.width() * 35 / 100)
    } else {
        (world.width() * 65 / 100, world.width() * 85 / 100)
    };
    let from_y = i32::from(world.surface);
    let to_y = world.height() - UNDERWORLD;
    if to_x <= from_x || to_y <= from_y {
        return None;
    }

    let mut left = ATTEMPTS;
    while left > 0 {
        let settle_for_mud = left < RELAX_TO_MUD;
        let ignore_air = left < RELAX_TO_ANYTHING;
        left -= 1;

        let x = rng.random_range(from_x..to_x);
        let mut y = rng.random_range(from_y..to_y);

        // Walk up the column until something jungle is under us.
        let mut climb = CLIMB;
        let mut found = false;
        while climb > 0 {
            climb -= 1;
            y -= 1;
            if y < from_y {
                break;
            }
            let tile = world.tile(x, y);
            if !(ignore_air || tile.is_active()) {
                continue;
            }
            if tile.block == JUNGLE_GRASS || (settle_for_mud && tile.block == MUD) {
                found = true;
                break;
            }
        }
        if !found {
            continue;
        }

        // A bulb sits on the tile above whatever we landed on, and will shuffle one either way.
        for at_x in [x, x - 1, x + 1] {
            if place(world, at_x, y - 1) {
                return Some((at_x, y - 1));
            }
        }
    }
    None
}

/// Put a bulb down, if it fits.
///
/// It is two tiles by two, anchored at its bottom left, and wants clear air with something solid
/// underneath — the same requirement any plant has.
pub fn place(world: &mut World, x: i32, y: i32) -> bool {
    if x < 2 || y < 2 || x + 1 >= world.width() - 2 || y >= world.height() - 2 {
        return false;
    }
    // The two-by-two it occupies has to be empty...
    for dx in 0..2 {
        for dy in 0..2 {
            if world.tile(x + dx, y - dy).is_active() {
                return false;
            }
        }
    }
    // ...and standing on jungle.
    for dx in 0..2 {
        let under = world.tile(x + dx, y + 1);
        if !under.is_active() || !matches!(under.block, JUNGLE_GRASS | MUD) {
            return false;
        }
    }

    for dx in 0..2i32 {
        for dy in 0..2i32 {
            let mut tile = world.tile(x + dx, y - dy);
            tile.block = BULB;
            tile.flags.set(terrustia_proto::TileFlags::ACTIVE, true);
            // Anchored bottom-left, so the frame counts up from the bottom row.
            tile.frame_x = (dx * 18) as i16;
            tile.frame_y = ((1 - dy) * 18) as i16;
            world.set_tile(x + dx, y - dy, tile);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use terrustia_proto::Tile;

    /// A patch of jungle with air above it, and a dungeon on the right so the band is on the left.
    fn jungle_world() -> World {
        let mut world = crate::world::worldgen::generate(1200, 600, "bulbs", 1);
        world.dungeon_x = Some(1000);
        for x in 180..420 {
            for y in 300..340 {
                world.set_tile(x, y, Tile::AIR);
            }
            for y in 340..360 {
                world.set_tile(x, y, Tile::block(JUNGLE_GRASS));
            }
        }
        world
    }

    /// A bulb goes into the jungle, on the ground, two tiles by two.
    #[test]
    fn a_bulb_grows_on_jungle_grass() {
        let mut world = jungle_world();
        let mut rng = SmallRng::seed_from_u64(7);
        let at = grow(&mut world, &mut rng).expect("nowhere in a jungle to put a bulb");

        assert_eq!(world.tile(at.0, at.1).block, BULB);
        assert_eq!(world.tile(at.0 + 1, at.1).block, BULB);
        assert_eq!(world.tile(at.0, at.1 - 1).block, BULB);
        assert_eq!(world.tile(at.0 + 1, at.1 - 1).block, BULB);
        assert_eq!(
            world.tile(at.0, at.1 + 1).block,
            JUNGLE_GRASS,
            "it should be standing on the jungle"
        );
    }

    /// A world with no jungle in it grows nothing, rather than putting a bulb in the rock.
    #[test]
    fn no_jungle_means_no_bulb() {
        let mut world = crate::world::worldgen::generate(1200, 600, "bare", 2);
        world.dungeon_x = Some(1000);
        let mut rng = SmallRng::seed_from_u64(7);
        assert!(grow(&mut world, &mut rng).is_none());
    }

    /// It will not put one on top of another, or into a wall.
    #[test]
    fn a_bulb_needs_room() {
        let mut world = jungle_world();
        assert!(place(&mut world, 300, 339), "clear ground should take one");
        assert!(
            !place(&mut world, 300, 339),
            "and should not take a second in the same place"
        );
        assert!(!place(&mut world, 300, 350), "nor one buried in the ground");
    }

    /// The band follows the jungle, which is always opposite the dungeon.
    #[test]
    fn bulbs_grow_on_the_side_the_jungle_is() {
        let mut rng = SmallRng::seed_from_u64(11);
        let mut world = jungle_world();
        // Dungeon on the right puts the search band on the left, where the jungle is.
        let at = grow(&mut world, &mut rng).expect("a bulb");
        assert!(at.0 < 600, "grew at {} with the dungeon on the right", at.0);
    }
}
