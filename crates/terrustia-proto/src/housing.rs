//! Housing rules for town NPCs, transcribed from `WorldGen.StartRoomCheck`, `CheckRoom` and
//! `RoomNeeds` in the 1.4.5.7 build.

/// Number of wall types in this build (`WallID.Count`).
pub const WALL_COUNT: u16 = 367;

/// Smallest room that counts as a house.
pub const MIN_ROOM_TILES: usize = 60;

/// Largest room the check will walk before giving up.
pub const MAX_ROOM_TILES: usize = 750;

/// Longest a room may be on either side.
pub const MAX_ROOM_SIZE: i32 = 100;

/// Walls that make a room count as enclosed. Natural walls like dirt do not.
const WALL_HOUSE: [u64; 6] = [
    0x1000FEFFEFFF1C72,
    0xFFFFFFF03F347F1C,
    0x05EBF3FFFFFFFFFF,
    0xFFEFFFFF00000000,
    0xFFFFFFFFFFFFFFFF,
    0x00007FFF9FFFFFFF,
];

/// Whether a wall type encloses a house.
pub const fn wall_encloses(wall: u16) -> bool {
    if wall == 0 || wall >= WALL_COUNT {
        return false;
    }
    WALL_HOUSE[(wall / 64) as usize] & (1u64 << (wall % 64)) != 0
}

/// Tiles that seal a room even though they are not solid.
pub fn housing_wall_tile(tile: u16) -> bool {
    matches!(tile, 11 | 386 | 389)
}

/// A room needs somewhere to sit.
pub fn counts_as_chair(tile: u16) -> bool {
    matches!(tile, 15 | 79 | 89 | 102 | 487 | 497)
}

/// A room needs a surface.
pub fn counts_as_table(tile: u16) -> bool {
    matches!(
        tile,
        14 | 18 | 87 | 88 | 90 | 101 | 354 | 355 | 464 | 469 | 487 | 699
    )
}

/// A room needs a light.
pub fn counts_as_torch(tile: u16) -> bool {
    matches!(
        tile,
        4 | 33
            | 34
            | 35
            | 42
            | 49
            | 92
            | 93
            | 95
            | 98
            | 100
            | 149
            | 173
            | 174
            | 270
            | 271
            | 316
            | 317
            | 318
            | 372
            | 405
            | 572
            | 581
            | 592
            | 646
            | 660
    )
}

/// A room needs a way in.
pub fn counts_as_door(tile: u16) -> bool {
    matches!(
        tile,
        10 | 11 | 19 | 386 | 387 | 388 | 389 | 427 | 435 | 436 | 437 | 438 | 439
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn furnishings_are_recognised() {
        assert!(counts_as_chair(15), "wooden chair");
        assert!(counts_as_table(14), "wooden table");
        assert!(counts_as_torch(4), "torch");
        assert!(counts_as_door(10), "closed door");
        assert!(counts_as_door(11), "open door");
        assert!(counts_as_door(19), "platforms count as a door");
    }

    #[test]
    fn plain_blocks_furnish_nothing() {
        for t in [0u16, 1, 2] {
            assert!(!counts_as_chair(t) && !counts_as_table(t));
            assert!(!counts_as_torch(t) && !counts_as_door(t));
        }
    }

    #[test]
    fn built_walls_enclose_and_natural_ones_do_not() {
        assert!(wall_encloses(4), "stone wall");
        assert!(!wall_encloses(0), "no wall at all");
        assert!(!wall_encloses(2), "natural dirt wall does not make a house");
    }

    #[test]
    fn out_of_range_walls_do_not_index_past_the_table() {
        assert!(!wall_encloses(WALL_COUNT));
        assert!(!wall_encloses(u16::MAX));
    }
}
