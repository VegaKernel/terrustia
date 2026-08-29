//! Early location checks for natural NPC spawning.
//!
//! Vanilla has two distinct coordinates here. It first chooses a random tile and rejects that tile
//! if it is solid or has a player-safe wall. For an ordinary non-Space attempt it then searches
//! downward for the physical `SpawnTileY` floor and only after that checks the left-shifted 2x3
//! clearance rectangle above the accepted floor. Keeping those stages separate is load-bearing when
//! the random tile is several rows above the ground.

use terrustia_proto::{Liquid, tile_solid::solid, wall_house};

use crate::world::World;

/// Whether the initially sampled tile may proceed to the downward floor search.
///
/// Only the random tile itself is inspected at this stage: it must not be solid and must not carry
/// a player-safe (`Main.wallHouse`) background wall. Clearance above the eventual floor is a later
/// check owned by [`floor_space_is_clear`].
pub fn random_candidate_is_valid(world: &World, x: i32, chosen_y: i32) -> bool {
    let chosen = world.tile(x, chosen_y);
    !(chosen.is_active() && solid(chosen.block)) && !wall_house::safe(chosen.wall)
}

/// Whether the vanilla-shaped 2x3 space immediately above an accepted physical floor is clear.
///
/// `x, floor_y` identify `SpawnTileX/SpawnTileY`, the solid tile the NPC will spawn above. The
/// rectangle is shifted one tile left: columns `x-1..=x`, rows `floor_y-3..=floor_y-1`. Any solid
/// tile, Lava, or cell outside the true world border rejects this location. Water, Honey and Shimmer
/// do not fail this early clearance; the later two-cell liquid postcheck owns the non-Water rule.
pub fn floor_space_is_clear(world: &World, x: i32, floor_y: i32) -> bool {
    if x <= 0 || x >= world.width() || floor_y < 3 || floor_y >= world.height() {
        return false;
    }

    for dx in -1..=0 {
        for dy in 1..=3 {
            let tile = world.tile(x + dx, floor_y - dy);
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

/// Temporary compatibility shape for the current caller.
///
/// The runtime still passes the same coordinate to both stages. This wrapper preserves that current
/// behavior while the helpers are reviewed independently; the follow-up `spawn.rs` change will call
/// [`random_candidate_is_valid`] before floor search and [`floor_space_is_clear`] afterward.
pub fn chosen_point_is_clear(world: &World, x: i32, chosen_y: i32) -> bool {
    random_candidate_is_valid(world, x, chosen_y) && floor_space_is_clear(world, x, chosen_y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::{Tile, tile_solid::solid_top};

    fn candidate() -> World {
        World::empty(100, 100, "spawn clearance")
    }

    #[test]
    fn clear_random_candidate_and_floor_space_are_independently_valid() {
        let world = candidate();
        assert!(random_candidate_is_valid(&world, 50, 30));
        assert!(floor_space_is_clear(&world, 50, 40));
    }

    #[test]
    fn solid_random_tile_rejects_before_floor_search() {
        let mut world = candidate();
        assert!(world.set_tile(50, 30, Tile::block(1)));
        assert!(!random_candidate_is_valid(&world, 50, 30));
    }

    #[test]
    fn safe_wall_at_random_tile_rejects_before_floor_search() {
        let mut world = candidate();
        let mut tile = Tile::AIR;
        tile.wall = 1; // Stone Wall: Main.wallHouse = true.
        assert!(world.set_tile(50, 30, tile));
        assert!(!random_candidate_is_valid(&world, 50, 30));
    }

    #[test]
    fn natural_unsafe_wall_does_not_reject_random_candidate() {
        let mut world = candidate();
        let mut tile = Tile::AIR;
        tile.wall = 62; // SpiderUnsafe: a natural wall, not a player-safe Spider Nest Wall.
        assert!(world.set_tile(50, 30, tile));
        assert!(random_candidate_is_valid(&world, 50, 30));
    }

    #[test]
    fn platform_like_random_tiles_are_solid_for_candidate_validation() {
        for block in [19, 239, 380] {
            let mut world = candidate();
            assert!(world.set_tile(50, 30, Tile::block(block)));
            assert!(solid(block), "tile {block} must be in Main.tileSolid");
            assert!(
                solid_top(block),
                "tile {block} must be marked solid-top/platform-like"
            );
            assert!(
                !random_candidate_is_valid(&world, 50, 30),
                "platform-like tile {block} must reject the random candidate"
            );
        }
    }

    #[test]
    fn obstruction_above_floor_does_not_retroactively_reject_random_air_tile() {
        let mut world = candidate();
        assert!(world.set_tile(49, 39, Tile::block(1)));
        assert!(random_candidate_is_valid(&world, 50, 30));
        assert!(!floor_space_is_clear(&world, 50, 40));
    }

    #[test]
    fn solid_in_left_column_above_floor_rejects_clearance() {
        let mut world = candidate();
        assert!(world.set_tile(49, 39, Tile::block(1)));
        assert!(!floor_space_is_clear(&world, 50, 40));
    }

    #[test]
    fn solid_on_top_clearance_row_rejects_clearance() {
        let mut world = candidate();
        assert!(world.set_tile(49, 37, Tile::block(1)));
        assert!(!floor_space_is_clear(&world, 50, 40));
    }

    #[test]
    fn lava_in_left_column_above_floor_rejects_clearance() {
        let mut world = candidate();
        assert!(world.set_tile(
            49,
            39,
            Tile::AIR.with_liquid(Liquid::Lava, 1)
        ));
        assert!(!floor_space_is_clear(&world, 50, 40));
    }

    #[test]
    fn water_above_floor_is_not_an_early_obstruction() {
        let mut world = candidate();
        assert!(world.set_tile(
            49,
            39,
            Tile::AIR.with_liquid(Liquid::Water, u8::MAX)
        ));
        assert!(floor_space_is_clear(&world, 50, 40));
    }

    #[test]
    fn honey_and_shimmer_are_left_for_the_later_liquid_postcheck() {
        for liquid in [Liquid::Honey, Liquid::Shimmer] {
            let mut world = candidate();
            assert!(world.set_tile(50, 39, Tile::AIR.with_liquid(liquid, 1)));
            assert!(floor_space_is_clear(&world, 50, 40));
        }
    }

    #[test]
    fn clearance_rejects_true_world_border_overflow() {
        let world = candidate();
        assert!(!floor_space_is_clear(&world, 0, 40));
        assert!(!floor_space_is_clear(&world, 50, 2));
        assert!(!floor_space_is_clear(&world, 100, 40));
    }

    #[test]
    fn compatibility_wrapper_keeps_current_pre_migration_behavior() {
        let mut world = candidate();
        assert!(chosen_point_is_clear(&world, 50, 40));
        assert!(world.set_tile(49, 39, Tile::block(1)));
        assert!(!chosen_point_is_clear(&world, 50, 40));
    }
}
