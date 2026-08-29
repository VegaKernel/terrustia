//! Clearance around a vanilla NPC spawn candidate.
//!
//! Natural spawning starts from a randomly chosen non-solid tile. Ground resolution is a later
//! stage: the game validates the chosen tile and a 2x3 rectangle immediately above it, then uses
//! the solid spawning tile found below to decide source/biome details. Keeping those coordinates
//! separate matters when the chosen point is several tiles above the ground.

use terrustia_proto::{Liquid, tile_solid::solid};

use crate::world::World;

/// Whether a chosen spawn point and the vanilla-shaped 2x3 rectangle above it are unobstructed.
///
/// `x, chosen_y` identify the random candidate tile before the downward ground scan. The chosen
/// tile itself must be non-solid. The clearance rectangle starts one row above it, covers that
/// column and the immediately-left column, and is three tiles high. Solid tiles or lava inside the
/// rectangle reject the candidate; water is allowed because liquid-source selection happens after
/// the spawning tile is resolved.
pub fn chosen_point_is_clear(world: &World, x: i32, chosen_y: i32) -> bool {
    let chosen = world.tile(x, chosen_y);
    if chosen.is_active() && solid(chosen.block) {
        return false;
    }

    for dx in -1..=0 {
        for dy in 1..=3 {
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
    fn solid_on_the_top_clearance_row_rejects_the_candidate() {
        let mut world = candidate();
        assert!(world.set_tile(49, 37, Tile::block(1)));
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
