//! Clearance around a vanilla NPC spawn candidate.
//!
//! Natural spawning starts from a randomly chosen air tile. Ground resolution is a later stage:
//! the game first validates a 2x3 rectangle at the chosen point, then uses the solid spawning tile
//! found below to decide the source/biome details. Keeping those coordinates separate matters when
//! the chosen point is several tiles above the ground.

use terrustia_proto::{Liquid, tile_solid::solid};

use crate::world::World;

/// Whether the vanilla-shaped 2x3 rectangle at a chosen spawn point is unobstructed.
///
/// `x, chosen_y` identify the random candidate tile before the downward ground scan. The rectangle
/// covers that column and the immediately-left column, three tiles high. Solid tiles and lava
/// reject the candidate; water is allowed because liquid source selection happens after ground is
/// resolved.
pub fn chosen_point_is_clear(world: &World, x: i32, chosen_y: i32) -> bool {
    for dx in -1..=0 {
        for dy in 0..3 {
            let tile = world.tile(x + dx, chosen_y - dy);
            if tile.is_active() && solid(tile.block) {
                return false;
            }
            if tile.liquid > 0 && tile.liquid_kind == Liquid::Lava {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::Tile;

    fn candidate() -> World {
        World::empty(100, 100, "spawn clearance")
    }

    #[test]
    fn clear_two_by_three_space_is_accepted_without_requiring_ground() {
        assert!(chosen_point_is_clear(&candidate(), 50, 40));
    }

    #[test]
    fn solid_chosen_tile_rejects_the_candidate() {
        let mut world = candidate();
        assert!(world.set_tile(50, 40, Tile::block(1)));
        assert!(!chosen_point_is_clear(&world, 50, 40));
    }

    #[test]
    fn solid_in_the_left_column_rejects_the_candidate() {
        let mut world = candidate();
        assert!(world.set_tile(49, 39, Tile::block(1)));
        assert!(!chosen_point_is_clear(&world, 50, 40));
    }

    #[test]
    fn lava_in_the_left_column_rejects_the_candidate() {
        let mut world = candidate();
        assert!(world.set_tile(
            49,
            39,
            Tile::AIR.with_liquid(Liquid::Lava, 1)
        ));
        assert!(!chosen_point_is_clear(&world, 50, 40));
    }

    #[test]
    fn water_in_the_left_column_is_not_an_obstruction() {
        let mut world = candidate();
        assert!(world.set_tile(
            49,
            39,
            Tile::AIR.with_liquid(Liquid::Water, u8::MAX)
        ));
        assert!(chosen_point_is_clear(&world, 50, 40));
    }

    #[test]
    fn ground_below_the_chosen_point_does_not_change_clearance() {
        let mut world = candidate();
        assert!(world.set_tile(50, 50, Tile::block(1)));
        assert!(chosen_point_is_clear(&world, 50, 40));
    }
}
