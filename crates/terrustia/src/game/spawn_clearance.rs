//! Clearance around a vanilla NPC spawning tile.
//!
//! The spawning tile is the solid tile chosen by the ground scan. Vanilla then validates a 2x3
//! rectangle immediately above it: the spawning column and the column immediately to its left,
//! three tiles high. A solid tile or lava anywhere in those six cells rejects the candidate.

use terrustia_proto::{Liquid, tile_solid::solid};

use crate::world::World;

/// Whether an NPC spawn has the vanilla-shaped 2x3 open rectangle above its solid floor tile.
///
/// `x, y` are the prospective NPC top-left tile coordinates used by `spawn::try_spawn`, so the
/// solid spawning tile itself is `(x, y + 1)`.
pub fn has_room(world: &World, x: i32, y: i32) -> bool {
    for dx in -1..=0 {
        for dy in 0..3 {
            let tile = world.tile(x + dx, y - dy);
            if tile.is_active() && solid(tile.block) {
                return false;
            }
            if tile.liquid > 0 && tile.liquid_kind == Liquid::Lava {
                return false;
            }
        }
    }

    let floor = world.tile(x, y + 1);
    floor.is_active() && solid(floor.block)
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::Tile;

    fn candidate() -> World {
        let mut world = World::empty(100, 100, "spawn clearance");
        assert!(world.set_tile(50, 50, Tile::block(1)));
        world
    }

    #[test]
    fn clear_two_by_three_space_is_accepted() {
        assert!(has_room(&candidate(), 50, 49));
    }

    #[test]
    fn solid_in_the_left_column_rejects_the_candidate() {
        let mut world = candidate();
        assert!(world.set_tile(49, 48, Tile::block(1)));
        assert!(!has_room(&world, 50, 49));
    }

    #[test]
    fn lava_in_the_left_column_rejects_the_candidate() {
        let mut world = candidate();
        assert!(world.set_tile(
            49,
            48,
            Tile::AIR.with_liquid(Liquid::Lava, 1)
        ));
        assert!(!has_room(&world, 50, 49));
    }

    #[test]
    fn water_in_the_left_column_is_not_an_obstruction() {
        let mut world = candidate();
        assert!(world.set_tile(
            49,
            48,
            Tile::AIR.with_liquid(Liquid::Water, u8::MAX)
        ));
        assert!(has_room(&world, 50, 49));
    }

    #[test]
    fn the_chosen_spawning_tile_must_still_be_solid() {
        let mut world = candidate();
        assert!(world.set_tile(50, 50, Tile::AIR));
        assert!(!has_room(&world, 50, 49));
    }
}
