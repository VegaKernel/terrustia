//! Effective ground tile type used by natural NPC source selection.
//!
//! The physical floor an NPC stands on is not always the tile type Terraria uses to choose the
//! NPC. Platforms, Planter Boxes and placed Metal Bars are `tileSolidTop`: vanilla looks through
//! them for a real solid source below. Conveyor Belts changed in 1.4.5 and instead use the tile
//! directly underneath the belt.

use terrustia_proto::tile_solid::{solid, solid_top};

use crate::world::World;

/// Platform-like source lookup reaches 29 tiles below the physical floor.
pub const SOURCE_SCAN_BELOW: i32 = 29;

const CONVEYOR_LEFT: u16 = 421;
const CONVEYOR_RIGHT: u16 = 422;

fn conveyor(block: u16) -> bool {
    matches!(block, CONVEYOR_LEFT | CONVEYOR_RIGHT)
}

/// Resolve the tile type Terraria uses as the ground/source for NPC selection.
///
/// `floor_y` is the physical solid tile found by the spawn-location search. An ordinary solid is
/// its own source. A Conveyor Belt used as the physical floor takes its source from exactly the tile
/// immediately underneath it. A solid-top floor instead searches downward through as many as 29
/// tiles and stops at the first solid block that is not itself solid-top.
///
/// If a floating solid-top tile has no qualifying block below it, retaining its own type is safer
/// than inventing a Dirt source from an inactive tile's zero-valued block field.
pub fn block(world: &World, x: i32, floor_y: i32) -> u16 {
    let floor = world.tile(x, floor_y);
    if conveyor(floor.block) {
        return world.tile(x, floor_y + 1).block;
    }
    if !solid_top(floor.block) {
        return floor.block;
    }

    for dy in 1..=SOURCE_SCAN_BELOW {
        let tile = world.tile(x, floor_y + dy);
        if !tile.is_active() || !solid(tile.block) || solid_top(tile.block) {
            continue;
        }
        return tile.block;
    }

    floor.block
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::Tile;

    fn world() -> World {
        World::empty(100, 100, "spawn source")
    }

    #[test]
    fn ordinary_solid_is_its_own_source() {
        let mut world = world();
        assert!(world.set_tile(50, 40, Tile::block(53)));
        assert_eq!(block(&world, 50, 40), 53);
    }

    #[test]
    fn platform_looks_through_to_the_solid_below() {
        let mut world = world();
        assert!(world.set_tile(50, 40, Tile::block(19)));
        assert!(world.set_tile(50, 47, Tile::block(53)));
        assert_eq!(block(&world, 50, 40), 53);
    }

    #[test]
    fn metal_bar_and_planter_box_use_the_same_solid_top_rule() {
        for floor in [239, 380] {
            let mut world = world();
            assert!(solid_top(floor), "{floor} must be solid-top in this build");
            assert!(world.set_tile(50, 40, Tile::block(floor)));
            assert!(world.set_tile(50, 45, Tile::block(60)));
            assert_eq!(block(&world, 50, 40), 60);
        }
    }

    #[test]
    fn stacked_solid_top_tiles_are_skipped() {
        let mut world = world();
        assert!(world.set_tile(50, 40, Tile::block(19)));
        assert!(world.set_tile(50, 44, Tile::block(380)));
        assert!(world.set_tile(50, 46, Tile::block(239)));
        assert!(world.set_tile(50, 49, Tile::block(1)));
        assert_eq!(block(&world, 50, 40), 1);
    }

    #[test]
    fn conveyor_floor_uses_exactly_the_tile_directly_below() {
        for belt in [CONVEYOR_LEFT, CONVEYOR_RIGHT] {
            let mut world = world();
            assert!(world.set_tile(50, 40, Tile::block(belt)));
            assert!(world.set_tile(50, 41, Tile::block(53)));
            assert!(world.set_tile(50, 45, Tile::block(1)));
            assert_eq!(block(&world, 50, 40), 53);
        }
    }

    #[test]
    fn platform_scan_stops_on_a_conveyor_without_reapplying_the_conveyor_rule() {
        let mut world = world();
        assert!(world.set_tile(50, 40, Tile::block(19)));
        assert!(world.set_tile(50, 45, Tile::block(CONVEYOR_LEFT)));
        assert!(world.set_tile(50, 46, Tile::block(53)));
        assert_eq!(block(&world, 50, 40), CONVEYOR_LEFT);
    }

    #[test]
    fn platform_scan_is_bounded_to_twenty_nine_tiles() {
        let mut world = world();
        assert!(world.set_tile(50, 40, Tile::block(19)));
        assert!(world.set_tile(50, 70, Tile::block(53)));
        assert_eq!(block(&world, 50, 40), 19);
    }
}
