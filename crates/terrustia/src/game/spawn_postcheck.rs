//! Post-selection checks for natural NPC spawn locations.
//!
//! These are deliberately separate from `spawn_clearance`: failing early clearance makes Terraria
//! try another random candidate, while these checks run after a candidate has already been accepted
//! and abort the current spawn attempt without retrying another point.

use terrustia_proto::Liquid;

use crate::world::World;

/// Vanilla's ordinary player hitbox size in pixels when not mounted.
pub const PLAYER_HITBOX_WIDTH: f32 = 20.0;
pub const PLAYER_HITBOX_HEIGHT: f32 = 42.0;

/// The post-selection rectangle around every player's hitbox center that a chosen tile must not
/// overlap. These are pixel dimensions, deliberately not rounded to whole tiles.
pub const PLAYER_VIEW_EXCLUSION_WIDTH: f32 = 2088.0;
pub const PLAYER_VIEW_EXCLUSION_HEIGHT: f32 = 1172.0;

/// Whether the 16x16 chosen-tile space is completely outside one player's exclusion rectangle.
///
/// `player_position` is Terraria's top-left entity position. Terrustia does not yet model mount-
/// specific hitbox dimensions, so this uses the ordinary 20x42 player hitbox and is exact for an
/// unmounted player. Touching an exclusion edge without overlapping it is allowed.
pub fn chosen_tile_outside_player_rectangle(
    chosen_x: i32,
    chosen_y: i32,
    player_position: (f32, f32),
) -> bool {
    let tile_left = chosen_x as f32 * 16.0;
    let tile_top = chosen_y as f32 * 16.0;
    let tile_right = tile_left + 16.0;
    let tile_bottom = tile_top + 16.0;

    let center_x = player_position.0 + PLAYER_HITBOX_WIDTH / 2.0;
    let center_y = player_position.1 + PLAYER_HITBOX_HEIGHT / 2.0;
    let exclusion_left = center_x - PLAYER_VIEW_EXCLUSION_WIDTH / 2.0;
    let exclusion_right = center_x + PLAYER_VIEW_EXCLUSION_WIDTH / 2.0;
    let exclusion_top = center_y - PLAYER_VIEW_EXCLUSION_HEIGHT / 2.0;
    let exclusion_bottom = center_y + PLAYER_VIEW_EXCLUSION_HEIGHT / 2.0;

    tile_right <= exclusion_left
        || tile_left >= exclusion_right
        || tile_bottom <= exclusion_top
        || tile_top >= exclusion_bottom
}

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
    fn chosen_tile_overlapping_player_rectangle_is_rejected() {
        // This places the exclusion rectangle's top-left at exactly pixel (0, 0).
        let player = (1034.0, 565.0);
        assert!(!chosen_tile_outside_player_rectangle(0, 0, player));
        assert!(!chosen_tile_outside_player_rectangle(130, 73, player));
    }

    #[test]
    fn chosen_tile_touching_or_beyond_the_rectangle_edge_is_allowed() {
        let player = (1034.0, 565.0);
        // x=-1 occupies [-16,0), touching the left edge only.
        assert!(chosen_tile_outside_player_rectangle(-1, 0, player));
        // The right edge is pixel 2088; x=130 overlaps it by 8 px, x=131 starts beyond it.
        assert!(!chosen_tile_outside_player_rectangle(130, 0, player));
        assert!(chosen_tile_outside_player_rectangle(131, 0, player));
        // The bottom edge is pixel 1172; y=73 overlaps by 4 px, y=74 starts beyond it.
        assert!(!chosen_tile_outside_player_rectangle(0, 73, player));
        assert!(chosen_tile_outside_player_rectangle(0, 74, player));
    }

    #[test]
    fn player_position_is_top_left_not_hitbox_center() {
        let player = (1034.0, 565.0);
        let center = (
            player.0 + PLAYER_HITBOX_WIDTH / 2.0,
            player.1 + PLAYER_HITBOX_HEIGHT / 2.0,
        );
        assert_eq!(center, (1044.0, 586.0));
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
