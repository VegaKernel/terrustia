//! Tile type property tables, transcribed from the 1.4.5.7 build.
//!
//! Two tables decide how a tile is encoded into a section, and getting either wrong corrupts the
//! stream in a way the client reports only by hanging:
//!
//! * [`frame_important`] decides whether `frameX`/`frameY` are written at all.
//! * [`allows_batching`] decides whether a tile may be run-length merged with its neighbour.

/// Number of tile types in this build (`TileID.Count`).
pub const TILE_COUNT: u16 = 754;

/// Bitset of types for which `Main.tileFrameImportant` is true.
///
/// Derived from the literal assignments in `Main.Initialize`, the `AddEchoFurnitureTile` helper
/// calls, and the 435..=439 loop.
const FRAME_IMPORTANT: [u64; 12] = [
    0x2086041EBD3FFC38,
    0x600647FFFFFEE780,
    0x0F0478200021EFF3,
    0x40FFFA981F968200,
    0xF47FFFFFEFF8E000,
    0x17F01FDC200EC019,
    0x3FF83B987C600FFC,
    0xE60A6FF118FFE3F0,
    0x3FB1DFFBC43EFFC0,
    0xA5E1FBFFFFFFFFF8,
    0xFDE0000003977FFD,
    0x00038000207B1FEF,
];

/// Whether a tile type stores `frameX`/`frameY` in the section stream.
///
/// Out-of-range types are treated as plain; a world holding one is already corrupt, and guessing
/// "framed" would desynchronise every tile after it.
pub const fn frame_important(tile: u16) -> bool {
    if tile >= TILE_COUNT {
        return false;
    }
    FRAME_IMPORTANT[(tile / 64) as usize] & (1u64 << (tile % 64)) != 0
}

/// Whether a tile type may be run-length batched with an identical neighbour.
///
/// `TileID.Sets.AllowsSaveCompressionBatching` defaults to true with exactly these exceptions.
pub const fn allows_batching(tile: u16) -> bool {
    !matches!(tile, 423 | 520 | 723 | 724)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_blocks_are_not_frame_important() {
        // Dirt, stone, grass, ores: no frames in the stream.
        for t in [0u16, 1, 2, 6, 7, 8, 9] {
            assert!(!frame_important(t), "type {t} should be plain");
        }
    }

    #[test]
    fn multi_tile_furniture_is_frame_important() {
        // Chests, doors, signs, workbenches.
        for t in [21u16, 10, 11, 55, 85, 88, 18] {
            assert!(frame_important(t), "type {t} should be framed");
        }
    }

    #[test]
    fn batching_exceptions_match_the_game() {
        for t in [423u16, 520, 723, 724] {
            assert!(!allows_batching(t));
        }
        for t in [0u16, 1, 2, 21, 519, 521] {
            assert!(allows_batching(t));
        }
    }

    #[test]
    fn out_of_range_types_do_not_index_past_the_table() {
        assert!(!frame_important(TILE_COUNT));
        assert!(!frame_important(u16::MAX));
    }
}

/// Whether a tile is sand, hardened sand or sandstone in any of its variants.
///
/// From `TileID.Sets.Conversion.{Sand, HardenedSand, Sandstone}`. A sand shark treats all three as
/// water to swim through, so a corrupt or hallowed desert is as swimmable as a clean one, and the
/// distinction between them is what stops one burrowing straight through a stone wall.
pub fn sandy(block: u16) -> bool {
    matches!(
        block,
        // Sand, ebonsand, pearlsand, crimsand.
        53 | 112 | 116 | 234
        // Hardened sand in the same four flavours.
        | 397 | 398 | 402 | 399
        // Sandstone likewise.
        | 396 | 400 | 403 | 401
    )
}

#[cfg(test)]
mod sand_tests {
    use super::sandy;

    /// The three families, and nothing outside them.
    #[test]
    fn the_sand_set_is_the_three_conversion_families() {
        for block in [53, 112, 116, 234, 397, 398, 402, 399, 396, 400, 403, 401] {
            assert!(sandy(block), "{block} should be sandy");
        }
        // Dirt, stone, snow and slush are not sand, however much they look like it underground.
        for block in [0, 1, 147, 224, 59, 60] {
            assert!(!sandy(block), "{block} should not be sandy");
        }
    }
}
