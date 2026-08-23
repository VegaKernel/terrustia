//! Which tile types block movement.
//!
//! Transcribed from `Main.tileSolid` and `Main.tileSolidTop` in the 1.4.5.7 build. Both are
//! filled by replaying its initialisation in order — the literal assignments *and* the loops that
//! set whole ranges, which is where the team blocks live and where a hand transcription lost
//! them.

use crate::tile_sets::TILE_COUNT;

/// Types that block movement from every side.
const SOLID: [u64; 12] = [
    0x9F61FBE042C807C7,
    0x8FF1B8000000185F,
    0xF0FB87DFFFDE1604,
    0xBF008D67E0095DFF,
    0x0B80000010071FFF,
    0xB80F80205E0003E6,
    0xC0FFCC67839FF01B,
    0x18F5901EF7001C01,
    0x004C20043BC0003F,
    0x0A1E040000000000,
    0x001FFFFFFD680002,
    0x00007FFFDFC4FF90,
];

/// Platforms: solid from above only, so an NPC can walk on one but also stand inside it.
const SOLID_TOP: [u64; 12] = [
    0x00000000000D4000,
    0x0004002001800000,
    0x0000000000000040,
    0x0000800000000000,
    0x00600F0063F80000,
    0x11001EC000080000,
    0x0000080060200780,
    0x0000000000200000,
    0x0000DEC144300000,
    0x0120081FFF800040,
    0x0000000000000039,
    0x0000000000000040,
];

/// Whether a tile type is in the game's solid set.
///
/// Platforms are in this set *and* in [`solid_top`]; collision has to check both, because a
/// platform only blocks something falling onto it from above.
pub const fn solid(tile: u16) -> bool {
    if tile >= TILE_COUNT {
        return false;
    }
    SOLID[(tile / 64) as usize] & (1u64 << (tile % 64)) != 0
}

/// Whether a tile type is a platform, solid only when landed on from above.
pub const fn solid_top(tile: u16) -> bool {
    if tile >= TILE_COUNT {
        return false;
    }
    SOLID_TOP[(tile / 64) as usize] & (1u64 << (tile % 64)) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_blocks_are_solid() {
        for t in [0u16, 1, 2, 30, 53] {
            assert!(solid(t), "tile {t} should be solid");
        }
    }

    #[test]
    fn decorations_are_not_solid() {
        // Torches, plants and cobwebs do not stop anything walking through.
        for t in [3u16, 4, 51] {
            assert!(!solid(t), "tile {t} should not be solid");
        }
    }

    #[test]
    fn platforms_are_marked_both_solid_and_solid_top() {
        // A platform sets *both* flags in the game. `solid_top` is what tells collision to let
        // something pass through from below and from the side, so movement code has to check it
        // rather than treating `solid` alone as "blocks me".
        assert!(solid(19), "wood platform is in the solid set");
        assert!(solid_top(19), "and is also marked solid-top");
        assert!(!solid_top(1), "plain stone is not a platform");
    }

    #[test]
    fn out_of_range_types_do_not_index_past_the_table() {
        assert!(!solid(TILE_COUNT));
        assert!(!solid(u16::MAX));
        assert!(!solid_top(u16::MAX));
    }
}

#[cfg(test)]
mod door_tests {
    use super::*;

    #[test]
    fn a_closed_door_blocks_and_an_open_one_does_not() {
        assert!(solid(10), "closed door must block movement");
        assert!(!solid_top(10), "a door is not a platform");
        assert!(!solid(11), "an open door lets you through");
        assert!(solid(388), "closed tall gate blocks");
    }
}
