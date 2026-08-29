//! Post-selection checks for natural NPC spawn locations.
//!
//! These are deliberately separate from `spawn_clearance`: failing early clearance makes Terraria
//! try another random candidate, while these checks run after a candidate has already been accepted
//! and abort the current spawn attempt without retrying another point.

use terrustia_proto::Liquid;

use crate::world::World;

/// Whether liquid in the two tiles directly above the chosen tile is allowed.
///
/// Dry tiles are fine. If either tile contains liquid, vanilla requires that liquid to be Water.
/// Honey, Lava and Shimmer therefore fail this post-selection check.
pub fn direct_above_liquid_is_water(world: &World, x: i32, chosen_y: i32) -> bool {
    (1..=2).all(|dy| {
        let tile = world.tile(x, chosen_y - dy);
        tile.liquid == 0 || tile.liquid_kind == Liquid::Water
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::Tile;

    fn world() -> World {
        World::empty(100, 100, "spawn postcheck")
    }

    #[test]
    fn dry_tiles_are_allowed() {
        assert!(direct_above_liquid_is_water(&world(), 50, 40));
    }

    #[test]
    fn water_in_either_or_both_directly_above_tiles_is_allowed() {
        for rows in [&[39][..], &[38][..], &[38, 39][..]] {
            let mut world = world();
            for &y in rows {
                assert!(world.set_tile(
                    50,
                    y,
                    Tile::AIR.with_liquid(Liquid::Water, u8::MAX)
                ));
            }
            assert!(direct_above_liquid_is_water(&world, 50, 40));
        }
    }

    #[test]
    fn honey_in_the_first_tile_above_fails() {
        let mut world = world();
        assert!(world.set_tile(
            50,
            39,
            Tile::AIR.with_liquid(Liquid::Honey, 1)
        ));
        assert!(!direct_above_liquid_is_water(&world, 50, 40));
    }

    #[test]
    fn shimmer_in_the_second_tile_above_fails() {
        let mut world = world();
        assert!(world.set_tile(
            50,
            38,
            Tile::AIR.with_liquid(Liquid::Shimmer, 1)
        ));
        assert!(!direct_above_liquid_is_water(&world, 50, 40));
    }

    #[test]
    fn lava_also_fails_the_postcheck() {
        let mut world = world();
        assert!(world.set_tile(
            50,
            39,
            Tile::AIR.with_liquid(Liquid::Lava, 1)
        ));
        assert!(!direct_above_liquid_is_water(&world, 50, 40));
    }

    #[test]
    fn liquid_three_tiles_above_is_outside_this_rule() {
        let mut world = world();
        assert!(world.set_tile(
            50,
            37,
            Tile::AIR.with_liquid(Liquid::Honey, 1)
        ));
        assert!(direct_above_liquid_is_water(&world, 50, 40));
    }
}
