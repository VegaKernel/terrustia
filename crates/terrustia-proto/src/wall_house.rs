//! Walls that suppress natural enemy spawning.
//!
//! Terraria exposes this as `Main.wallHouse`: a wall whose entry is true is considered "safe" for
//! enemy-spawning purposes. This table is pinned to the 1.4.5 wall id space used by Terrustia
//! (0..=366). ID 0 means no wall and is deliberately not safe.
//!
//! The values were cross-checked against the current official Terraria Wall IDs table, including
//! the 1.4.5 additions. Natural/unsafe variants remain false while their player-safe or Echo
//! counterparts remain true.

/// `WallID.Count` for the 1.4.5 wall id space consumed by this server.
pub const WALL_COUNT: u16 = 367;

/// `Main.wallHouse`, packed 64 wall ids per word.
const SAFE: [u64; 6] = [
    0x1000FEFFEFFF1C72,
    0xFFFFFFF03F347F1C,
    0x05EBF3FFFFFFFFFF,
    0xFFEFFFFF80000000,
    0xFFFFFFFFFFFFFFFF,
    0x00007FFF9FFFFFFF,
];

/// Whether a wall id is considered safe for natural enemy spawning.
pub const fn safe(wall: u16) -> bool {
    if wall >= WALL_COUNT {
        return false;
    }
    SAFE[(wall / 64) as usize] & (1u64 << (wall % 64)) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_player_walls_are_safe() {
        for wall in [1, 4, 16, 17, 66, 108, 112, 236, 304] {
            assert!(safe(wall), "wall {wall} should be safe");
        }
    }

    #[test]
    fn natural_wall_variants_are_not_safe() {
        for wall in [2, 7, 40, 62, 86, 87, 94, 170, 178, 187, 216, 244] {
            assert!(!safe(wall), "wall {wall} should remain unsafe");
        }
    }

    #[test]
    fn one_four_five_additions_keep_their_safe_wall_flags() {
        for wall in [347, 348, 351, 352, 365, 366] {
            assert!(safe(wall), "1.4.5 wall {wall} should be safe");
        }
        for wall in [349, 350] {
            assert!(!safe(wall), "1.4.5 wall {wall} should be unsafe");
        }
    }

    #[test]
    fn no_wall_and_out_of_range_ids_are_not_safe() {
        assert!(!safe(0));
        assert!(!safe(WALL_COUNT));
        assert!(!safe(u16::MAX));
    }
}
