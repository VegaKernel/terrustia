//! Screen-relative bounds used by natural NPC spawning.
//!
//! Terraria's integer tile rectangles are deliberately asymmetric because the random upper bound
//! is exclusive. Treating the documented horizontal/vertical ranges as `±N` expands the east/down
//! edge by one tile and, for the vertical spawn range, used to expand both sides in Terrustia.

use rand::{Rng, rngs::SmallRng};

/// Maximum offset of the random chosen tile from the player's top-left hitbox tile.
pub const SPAWN_WEST: i32 = 84;
pub const SPAWN_EAST: i32 = 83;
pub const SPAWN_UP: i32 = 46;
pub const SPAWN_DOWN: i32 = 45;

/// Bounds of the early safe rectangle around the player's top-left hitbox tile.
pub const SAFE_WEST: i32 = 62;
pub const SAFE_EAST: i32 = 61;
pub const SAFE_UP: i32 = 35;
pub const SAFE_DOWN: i32 = 34;

/// Compatibility widths used by callers/documentation that describe the vanilla half-open ranges.
pub const SPAWN_RANGE_X: i32 = SPAWN_WEST;
pub const SPAWN_RANGE_Y: i32 = SPAWN_UP;
pub const SAFE_RANGE_X: i32 = SAFE_WEST;
pub const SAFE_RANGE_Y: i32 = SAFE_UP;

/// Pick one vanilla-sized random tile offset.
pub fn choose_offset(rng: &mut SmallRng) -> (i32, i32) {
    (
        rng.random_range(-SPAWN_WEST..=SPAWN_EAST),
        rng.random_range(-SPAWN_UP..=SPAWN_DOWN),
    )
}

/// Exclusive bottom row of the normal spawn area for a player's top-left hitbox tile row.
///
/// Candidate Y sampling is `player_y-46 .. player_y+46` (upper bound exclusive), so the final row
/// inside that same normal spawn area is `player_y+45`. Ordinary non-Space floor search must not
/// invent a separate fixed depth: it searches below the sampled tile only while it remains inside
/// this area.
pub const fn normal_spawn_bottom_exclusive(player_y: i32) -> i32 {
    player_y + SPAWN_DOWN + 1
}

/// Whether the resolved solid floor lies in the early player-safe rectangle.
///
/// Both offsets are relative to the tile containing the top-left corner of the player's hitbox.
/// The caller must pass the solid floor row, not the NPC's stand/top-left row one tile above it.
pub fn in_safe_rectangle(dx: i32, dy: i32) -> bool {
    (-SAFE_WEST..=SAFE_EAST).contains(&dx) && (-SAFE_UP..=SAFE_DOWN).contains(&dy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn spawn_rectangle_has_the_vanilla_asymmetric_edges() {
        assert_eq!((-SPAWN_WEST, SPAWN_EAST), (-84, 83));
        assert_eq!((-SPAWN_UP, SPAWN_DOWN), (-46, 45));
    }

    #[test]
    fn normal_floor_search_uses_the_spawn_area_bottom_not_a_fixed_depth() {
        let player_y = 100;
        assert_eq!(normal_spawn_bottom_exclusive(player_y), 146);
        assert_eq!(normal_spawn_bottom_exclusive(player_y) - 1, player_y + SPAWN_DOWN);

        // A candidate at the top of the range can therefore search much farther than 30 rows.
        let top_candidate = player_y - SPAWN_UP;
        assert_eq!(top_candidate, 54);
        assert_eq!(normal_spawn_bottom_exclusive(player_y) - top_candidate, 92);
    }

    #[test]
    fn safe_rectangle_includes_each_vanilla_edge_and_rejects_the_next_tile() {
        for (dx, dy) in [
            (-SAFE_WEST, 0),
            (SAFE_EAST, 0),
            (0, -SAFE_UP),
            (0, SAFE_DOWN),
        ] {
            assert!(
                in_safe_rectangle(dx, dy),
                "edge ({dx}, {dy}) should be safe"
            );
        }
        for (dx, dy) in [
            (-SAFE_WEST - 1, 0),
            (SAFE_EAST + 1, 0),
            (0, -SAFE_UP - 1),
            (0, SAFE_DOWN + 1),
        ] {
            assert!(
                !in_safe_rectangle(dx, dy),
                "outside tile ({dx}, {dy}) should not be safe"
            );
        }
    }

    #[test]
    fn random_offsets_never_escape_the_rectangle() {
        let mut rng = SmallRng::seed_from_u64(7);
        for _ in 0..100_000 {
            let (dx, dy) = choose_offset(&mut rng);
            assert!((-SPAWN_WEST..=SPAWN_EAST).contains(&dx));
            assert!((-SPAWN_UP..=SPAWN_DOWN).contains(&dy));
        }
    }
}
