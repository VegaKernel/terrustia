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

const MOWED_GRASS: u16 = 477;
const MOWED_HALLOWED_GRASS: u16 = 492;

/// Events that disable mowed grass's ordinary 1/10 natural-spawn rejection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MowedGrassEvents {
    pub blood_moon: bool,
    pub eclipse: bool,
    pub pumpkin_moon: bool,
    pub frost_moon: bool,
    pub slime_rain: bool,
    pub invasion: bool,
}

impl MowedGrassEvents {
    pub const fn any(self) -> bool {
        self.blood_moon
            || self.eclipse
            || self.pumpkin_moon
            || self.frost_moon
            || self.slime_rain
            || self.invasion
    }
}

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

/// Whether a resolved spawn source satisfies vanilla's Dungeon post-check.
///
/// Outside the Dungeon this check is irrelevant. Inside it, the resolved source must be one of the
/// three ordinary or three cracked Dungeon Brick tile types and the wall immediately above that
/// source must be non-zero. Vanilla accepts any wall here, including a player-safe wall.
pub fn dungeon_source_is_valid(
    player_in_dungeon: bool,
    spawn_tile_type: u16,
    spawn_wall_type: u16,
) -> bool {
    if !player_in_dungeon {
        return true;
    }
    matches!(spawn_tile_type, 41 | 43 | 44 | 481 | 482 | 483) && spawn_wall_type != 0
}

/// Whether mowed grass rejects an otherwise-valid spawn attempt.
///
/// Vanilla makes exactly a one-in-ten roll on Mowed grass / Mowed Hallowed grass when none of the
/// six listed events are active. The roll is provided lazily so callers do not consume RNG at all
/// for another source tile or while an event disables this rule.
pub fn mowed_grass_rejects(
    spawn_tile_type: u16,
    events: MowedGrassEvents,
    one_in_ten_roll: impl FnOnce() -> bool,
) -> bool {
    matches!(spawn_tile_type, MOWED_GRASS | MOWED_HALLOWED_GRASS)
        && !events.any()
        && one_in_ten_roll()
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
    fn dungeon_postcheck_accepts_all_six_dungeon_brick_types_with_a_wall() {
        for block in [41, 43, 44, 481, 482, 483] {
            assert!(dungeon_source_is_valid(true, block, 1), "Dungeon brick {block}");
        }
    }

    #[test]
    fn dungeon_postcheck_requires_both_dungeon_brick_and_a_wall() {
        assert!(!dungeon_source_is_valid(true, 41, 0));
        assert!(!dungeon_source_is_valid(true, 1, 1));
        assert!(!dungeon_source_is_valid(true, 1, 0));
        // Outside a Dungeon, neither restriction is part of this post-check.
        assert!(dungeon_source_is_valid(false, 1, 0));
    }

    #[test]
    fn every_documented_event_disables_mowed_grass_rejection() {
        let cases = [
            MowedGrassEvents { blood_moon: true, ..Default::default() },
            MowedGrassEvents { eclipse: true, ..Default::default() },
            MowedGrassEvents { pumpkin_moon: true, ..Default::default() },
            MowedGrassEvents { frost_moon: true, ..Default::default() },
            MowedGrassEvents { slime_rain: true, ..Default::default() },
            MowedGrassEvents { invasion: true, ..Default::default() },
        ];
        for events in cases {
            assert!(!mowed_grass_rejects(MOWED_GRASS, events, || true));
            assert!(!mowed_grass_rejects(MOWED_HALLOWED_GRASS, events, || true));
        }
    }

    #[test]
    fn mowed_grass_rejects_only_on_the_one_in_ten_roll() {
        let quiet = MowedGrassEvents::default();
        assert!(mowed_grass_rejects(MOWED_GRASS, quiet, || true));
        assert!(mowed_grass_rejects(MOWED_HALLOWED_GRASS, quiet, || true));
        assert!(!mowed_grass_rejects(MOWED_GRASS, quiet, || false));
        assert!(!mowed_grass_rejects(2, quiet, || true));
        assert!(!mowed_grass_rejects(109, quiet, || true));
    }

    #[test]
    fn irrelevant_mowed_grass_rolls_do_not_consume_rng() {
        let quiet = MowedGrassEvents::default();
        let mut rolled = false;
        assert!(!mowed_grass_rejects(2, quiet, || {
            rolled = true;
            true
        }));
        assert!(!rolled);

        let mut rolled = false;
        assert!(!mowed_grass_rejects(
            MOWED_GRASS,
            MowedGrassEvents { invasion: true, ..Default::default() },
            || {
                rolled = true;
                true
            }
        ));
        assert!(!rolled);
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
