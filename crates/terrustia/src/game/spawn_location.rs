//! Ordinary natural-NPC spawn-location resolution.
//!
//! Terraria's normal (non-Space) path has two different Y coordinates that are easy to conflate:
//! a random candidate inside the spawn rectangle, then the physical solid `SpawnTileY` found below
//! it. Retryable location failures happen while finding that floor; once a floor survives the early
//! checks, visibility/liquid failures abort the current spawn attempt instead of trying another
//! random point.

use terrustia_proto::tile_solid::solid;

use crate::world::World;

/// Result of evaluating one random candidate in the normal natural-spawn path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalCandidate {
    /// This random point is unsuitable, but vanilla may try another one from the same 50-attempt
    /// search.
    Retry,
    /// A physical floor was accepted, but a post-selection check failed. Vanilla abandons this
    /// spawn attempt rather than searching another random point.
    Abort,
    /// The accepted physical floor (`SpawnTileY`), i.e. the tile the NPC will spawn above.
    Accept { floor_y: i32 },
}

/// Resolve one random point through the ordinary non-Space `FindSpawnTile`-shaped pipeline.
///
/// `player_tile` is the spawning player's top-left hitbox tile coordinate. `active_players` contains
/// every active player's top-left pixel position because the post-selection visibility rectangle is
/// global, not just relative to the player whose spawn roll is being processed.
pub fn evaluate_normal_candidate(
    world: &World,
    player_tile: (i32, i32),
    active_players: &[(f32, f32)],
    x: i32,
    random_y: i32,
) -> NormalCandidate {
    if !crate::game::spawn_clearance::random_candidate_is_valid(world, x, random_y) {
        return NormalCandidate::Retry;
    }

    let bottom = crate::game::spawn_ranges::normal_spawn_bottom_exclusive(player_tile.1)
        .min(world.height());
    if random_y >= bottom {
        return NormalCandidate::Retry;
    }

    let Some(floor_y) = (random_y..bottom).find(|&y| {
        let tile = world.tile(x, y);
        tile.is_active() && solid(tile.block)
    }) else {
        return NormalCandidate::Retry;
    };

    if crate::game::spawn_ranges::in_safe_rectangle(x - player_tile.0, floor_y - player_tile.1) {
        return NormalCandidate::Retry;
    }

    if !crate::game::spawn_clearance::floor_space_is_clear(world, x, floor_y) {
        return NormalCandidate::Retry;
    }

    if active_players.iter().any(|&position| {
        !crate::game::spawn_postcheck::chosen_tile_outside_player_rectangle(x, floor_y, position)
    }) {
        return NormalCandidate::Abort;
    }

    if !crate::game::spawn_postcheck::direct_above_liquid_is_water(world, x, floor_y) {
        return NormalCandidate::Abort;
    }

    NormalCandidate::Accept { floor_y }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::{Liquid, Tile};

    fn world() -> World {
        World::empty(300, 300, "spawn location")
    }

    #[test]
    fn top_edge_candidate_can_find_floor_far_beyond_the_old_thirty_tile_scan() {
        let mut world = world();
        let player = (150, 100);
        let random_y = player.1 - crate::game::spawn_ranges::SPAWN_UP;
        assert_eq!(random_y, 54);

        // Still inside the normal spawn rectangle, but 91 rows below the random candidate.
        assert!(world.set_tile(80, 145, Tile::block(1)));
        assert_eq!(
            evaluate_normal_candidate(&world, player, &[], 80, random_y),
            NormalCandidate::Accept { floor_y: 145 }
        );
    }

    #[test]
    fn floor_at_the_exclusive_spawn_bottom_is_not_eligible() {
        let mut world = world();
        let player = (150, 100);
        let random_y = player.1 - crate::game::spawn_ranges::SPAWN_UP;
        let bottom = crate::game::spawn_ranges::normal_spawn_bottom_exclusive(player.1);
        assert_eq!(bottom, 146);
        assert!(world.set_tile(80, bottom, Tile::block(1)));
        assert_eq!(
            evaluate_normal_candidate(&world, player, &[], 80, random_y),
            NormalCandidate::Retry
        );
    }

    #[test]
    fn safe_random_wall_retries_before_floor_search() {
        let mut world = world();
        let mut tile = Tile::AIR;
        tile.wall = 1; // Stone Wall: Main.wallHouse = true.
        assert!(world.set_tile(80, 60, tile));
        assert!(world.set_tile(80, 90, Tile::block(1)));
        assert_eq!(
            evaluate_normal_candidate(&world, (150, 100), &[], 80, 60),
            NormalCandidate::Retry
        );
    }

    #[test]
    fn floor_inside_early_safe_rectangle_retries() {
        let mut world = world();
        assert!(world.set_tile(100, 120, Tile::block(1)));
        assert_eq!(
            evaluate_normal_candidate(&world, (100, 100), &[], 100, 90),
            NormalCandidate::Retry
        );
    }

    #[test]
    fn obstruction_above_resolved_floor_retries_not_aborts() {
        let mut world = world();
        assert!(world.set_tile(80, 120, Tile::block(1)));
        assert!(world.set_tile(79, 119, Tile::block(1)));
        assert_eq!(
            evaluate_normal_candidate(&world, (150, 100), &[], 80, 90),
            NormalCandidate::Retry
        );
    }

    #[test]
    fn overlap_with_any_active_player_aborts_after_floor_selection() {
        let mut world = world();
        assert!(world.set_tile(80, 120, Tile::block(1)));
        let player_position = (80.0 * 16.0, 120.0 * 16.0);
        assert_eq!(
            evaluate_normal_candidate(&world, (150, 100), &[player_position], 80, 90),
            NormalCandidate::Abort
        );
    }

    #[test]
    fn non_water_liquid_directly_above_floor_aborts_without_retry() {
        for liquid in [Liquid::Honey, Liquid::Shimmer] {
            let mut world = world();
            assert!(world.set_tile(80, 120, Tile::block(1)));
            assert!(world.set_tile(80, 119, Tile::AIR.with_liquid(liquid, 1)));
            assert_eq!(
                evaluate_normal_candidate(&world, (150, 100), &[], 80, 90),
                NormalCandidate::Abort
            );
        }
    }

    #[test]
    fn water_directly_above_floor_remains_valid() {
        let mut world = world();
        assert!(world.set_tile(80, 120, Tile::block(1)));
        assert!(world.set_tile(
            80,
            119,
            Tile::AIR.with_liquid(Liquid::Water, u8::MAX)
        ));
        assert_eq!(
            evaluate_normal_candidate(&world, (150, 100), &[], 80, 90),
            NormalCandidate::Accept { floor_y: 120 }
        );
    }
}
