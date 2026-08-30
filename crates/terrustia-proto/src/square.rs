//! Packet `20`, `AreaTileChange` — a small rectangle of tiles pushed as a unit.
//!
//! Clients send this whenever a change spans more than one tile: placing furniture, opening a
//! door, growing a tree. Applying it is what lets a server stay in step with multi-tile operations
//! without reimplementing the game's placement and framing rules.
//!
//! The encoding is denser than a section's and completely different: three flag bytes per tile, no
//! run-length, and tiles walked **column by column** within the rectangle.
//!
//! Applying a decoded square is a *merge* onto whatever tile is already there, not a fresh
//! overwrite — see [`TileSquare::decode`]'s own doc for the field-by-field rules, transcribed
//! from `MessageBuffer.cs:1358-1437`.

use crate::{
    error::{ProtoError, Result},
    id,
    reader::PacketReader,
    tile::{Liquid, Tile, TileFlags},
    tile_sets::frame_important,
    writer::{PacketWriter, Writer},
};

/// The rectangle is addressed with byte dimensions, so it can never exceed 255 on a side.
pub const MAX_SIDE: usize = 255;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileSquare {
    pub x: i16,
    pub y: i16,
    pub width: u8,
    pub height: u8,
    /// `TileChangeType`; 0 means an ordinary edit.
    pub change_type: u8,
    /// Tiles in column-major order: all of column 0 top to bottom, then column 1.
    pub tiles: Vec<Tile>,
}

impl TileSquare {
    pub fn tile_count(&self) -> usize {
        usize::from(self.width) * usize::from(self.height)
    }

    /// The tile at an offset within the square.
    pub fn tile(&self, dx: usize, dy: usize) -> Option<Tile> {
        if dx >= usize::from(self.width) || dy >= usize::from(self.height) {
            return None;
        }
        self.tiles.get(dx * usize::from(self.height) + dy).copied()
    }

    /// Decode a packet `20` payload against the tiles already on the ground.
    ///
    /// Vanilla's own receive handler (`MessageBuffer.cs:1358-1437`) never builds a tile from
    /// scratch: `tile4 = Main.tile[x, y]` is the *existing* tile, and every field below mutates it
    /// in place, some unconditionally, some only when the packet's bits say so. `existing_tile`
    /// is how the caller supplies that starting point — the world position, not an index into
    /// this square, since the packet is walked column by column starting at `(x, y)`.
    pub fn decode(payload: &[u8], existing_tile: impl Fn(i32, i32) -> Tile) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        let x = r.i16()?;
        let y = r.i16()?;
        let width = r.u8()?;
        let height = r.u8()?;
        let change_type = r.u8()?;

        let count = usize::from(width) * usize::from(height);
        let mut tiles = vec![Tile::AIR; count];
        // Column by column, outer x then inner y — `MessageBuffer.cs:1359-1361`'s own nested
        // loop, which is also this file's own on-wire order (see `tile`'s doc above).
        for dx in 0..usize::from(width) {
            for dy in 0..usize::from(height) {
                let existing = existing_tile(i32::from(x) + dx as i32, i32::from(y) + dy as i32);
                tiles[dx * usize::from(height) + dy] = read_tile(&mut r, existing)?;
            }
        }

        Ok(Self {
            x,
            y,
            width,
            height,
            change_type,
            tiles,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.tiles.len() != self.tile_count() {
            return Err(ProtoError::OutOfRange {
                field: "tile square size",
                value: self.tiles.len() as i64,
            });
        }
        let mut w = PacketWriter::new(id::AREA_TILE_CHANGE);
        w.i16(self.x)
            .i16(self.y)
            .u8(self.width)
            .u8(self.height)
            .u8(self.change_type);
        for tile in &self.tiles {
            write_tile(&mut w, tile);
        }
        w.finish()
    }
}

/// Apply one tile's worth of packet `20` onto the tile already there, field by field, exactly the
/// way `MessageBuffer.cs:1358-1437` does it. This is a *merge*, not a fresh decode: colour, wall
/// colour, and liquid are deliberately left untouched when their bit is absent (a real client
/// never sets the liquid bit on send at all, `NetMessage.cs:593`, so a dedicated server never
/// overwrites liquid from this packet in practice), `wall`'s absent bit means something
/// different — "no wall here" — and is honoured by clearing it, and `block`/`frame_x`/`frame_y`/
/// `slope` only change at all when the `active` bit that arrived is set.
fn read_tile(r: &mut PacketReader<'_>, existing: Tile) -> Result<Tile> {
    let flags1 = r.u8()?;
    let flags2 = r.u8()?;
    let flags3 = r.u8()?;

    let mut tile = existing;

    // Captured before `active` is overwritten below — vanilla's `flag8`, used later to decide
    // whether a frame reset is owed (`MessageBuffer.cs:1368`).
    let was_active = tile.is_active();

    let active = flags1 & 0x01 != 0;
    let has_wall = flags1 & 0x04 != 0;
    let has_liquid = flags1 & 0x08 != 0;

    tile.flags.set(TileFlags::ACTIVE, active);
    // Wall presence is not "preserved when absent" the way liquid and paint are below: vanilla
    // always resolves it this packet, clearing to no-wall when the bit is unset and reading the
    // real id when it is set (`MessageBuffer.cs:1373`, `:1427-1430`).
    tile.wall = 0;

    tile.flags.set(TileFlags::WIRE_RED, flags1 & 0x10 != 0);
    tile.flags.set(TileFlags::HALF_BRICK, flags1 & 0x20 != 0);
    tile.flags.set(TileFlags::ACTUATOR, flags1 & 0x40 != 0);
    tile.flags.set(TileFlags::ACTUATED, flags1 & 0x80 != 0);
    tile.flags.set(TileFlags::WIRE_BLUE, flags2 & 0x01 != 0);
    tile.flags.set(TileFlags::WIRE_GREEN, flags2 & 0x02 != 0);
    tile.flags.set(TileFlags::WIRE_YELLOW, flags2 & 0x80 != 0);

    // Left alone — not zeroed — when the bit is absent: an existing colour survives a square that
    // says nothing about colour (`MessageBuffer.cs:1385-1392`).
    if flags2 & 0x04 != 0 {
        tile.color = r.u8()?;
    }
    if flags2 & 0x08 != 0 {
        tile.wall_color = r.u8()?;
    }

    if active {
        let old_block = tile.block;
        // Unlike a section, the type is always two bytes here.
        tile.block = r.u16()?;
        if frame_important(tile.block) {
            tile.frame_x = r.i16()?;
            tile.frame_y = r.i16()?;
        } else if !was_active || tile.block != old_block {
            // Turning active for the first time, or changing type without new frame data of its
            // own: the old frame no longer describes anything real (`MessageBuffer.cs:1402-1406`).
            tile.frame_x = -1;
            tile.frame_y = -1;
        }
        // The slope is spread across three separate bits rather than a packed field.
        let mut slope = 0u8;
        if flags2 & 0x10 != 0 {
            slope += 1;
        }
        if flags2 & 0x20 != 0 {
            slope += 2;
        }
        if flags2 & 0x40 != 0 {
            slope += 4;
        }
        tile.slope = slope;
    }
    // When `active` is false, block/frame/slope are left exactly as the existing tile had them —
    // vanilla's own `if (tile4.active())` guard around all four (`MessageBuffer.cs:1393-1421`).

    tile.flags
        .set(TileFlags::FULLBRIGHT_BLOCK, flags3 & 0x01 != 0);
    tile.flags
        .set(TileFlags::FULLBRIGHT_WALL, flags3 & 0x02 != 0);
    tile.flags
        .set(TileFlags::INVISIBLE_BLOCK, flags3 & 0x04 != 0);
    tile.flags
        .set(TileFlags::INVISIBLE_WALL, flags3 & 0x08 != 0);

    if has_wall {
        tile.wall = r.u16()?;
    }
    // Left alone when the bit is absent, same as colour above — and on a real dedicated server
    // that bit is always absent (see the doc comment above). This is the fix for the
    // liquid-erasing bug: the old code decoded straight into a fresh `Tile::AIR` and always
    // overwrote `liquid` (leaving it `0` whenever this bit was unset), deleting any pre-existing
    // liquid on every ordinary tile square a client ever sent.
    if has_liquid {
        tile.liquid = r.u8()?;
        tile.liquid_kind = match r.u8()? {
            1 => Liquid::Lava,
            2 => Liquid::Honey,
            3 => Liquid::Shimmer,
            _ => Liquid::Water,
        };
    }

    Ok(tile)
}

fn write_tile(w: &mut Writer, tile: &Tile) {
    let active = tile.is_active();
    let has_wall = tile.wall != 0;
    let has_liquid = tile.liquid != 0;

    let mut flags1 = 0u8;
    if active {
        flags1 |= 0x01;
    }
    if has_wall {
        flags1 |= 0x04;
    }
    if has_liquid {
        flags1 |= 0x08;
    }
    if tile.flags.has(TileFlags::WIRE_RED) {
        flags1 |= 0x10;
    }
    if tile.flags.has(TileFlags::HALF_BRICK) {
        flags1 |= 0x20;
    }
    if tile.flags.has(TileFlags::ACTUATOR) {
        flags1 |= 0x40;
    }
    if tile.flags.has(TileFlags::ACTUATED) {
        flags1 |= 0x80;
    }

    let mut flags2 = 0u8;
    if tile.flags.has(TileFlags::WIRE_BLUE) {
        flags2 |= 0x01;
    }
    if tile.flags.has(TileFlags::WIRE_GREEN) {
        flags2 |= 0x02;
    }
    if tile.color != 0 {
        flags2 |= 0x04;
    }
    if tile.wall_color != 0 {
        flags2 |= 0x08;
    }
    if active {
        if tile.slope & 1 != 0 {
            flags2 |= 0x10;
        }
        if tile.slope & 2 != 0 {
            flags2 |= 0x20;
        }
        if tile.slope & 4 != 0 {
            flags2 |= 0x40;
        }
    }
    if tile.flags.has(TileFlags::WIRE_YELLOW) {
        flags2 |= 0x80;
    }

    let mut flags3 = 0u8;
    if tile.flags.has(TileFlags::FULLBRIGHT_BLOCK) {
        flags3 |= 0x01;
    }
    if tile.flags.has(TileFlags::FULLBRIGHT_WALL) {
        flags3 |= 0x02;
    }
    if tile.flags.has(TileFlags::INVISIBLE_BLOCK) {
        flags3 |= 0x04;
    }
    if tile.flags.has(TileFlags::INVISIBLE_WALL) {
        flags3 |= 0x08;
    }

    w.u8(flags1).u8(flags2).u8(flags3);
    if tile.color != 0 {
        w.u8(tile.color);
    }
    if tile.wall_color != 0 {
        w.u8(tile.wall_color);
    }
    if active {
        w.u16(tile.block);
        if frame_important(tile.block) {
            w.i16(tile.frame_x).i16(tile.frame_y);
        }
    }
    if has_wall {
        w.u16(tile.wall);
    }
    if has_liquid {
        let kind = match tile.liquid_kind {
            Liquid::Water => 0,
            Liquid::Lava => 1,
            Liquid::Honey => 2,
            Liquid::Shimmer => 3,
        };
        w.u8(tile.liquid).u8(kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square_of(tiles: Vec<Tile>, width: u8, height: u8) -> TileSquare {
        TileSquare {
            x: 100,
            y: 200,
            width,
            height,
            change_type: 0,
            tiles,
        }
    }

    /// Decodes over bare air at every position, which is what makes these round-trip tests valid:
    /// `encode` always carries every non-default field explicitly, so merging onto `Tile::AIR`
    /// reconstructs the original tile exactly, the same as the old "decode into a fresh tile"
    /// behaviour did.
    fn round_trip(square: &TileSquare) -> TileSquare {
        round_trip_over(square, |_, _| Tile::AIR)
    }

    fn round_trip_over(square: &TileSquare, existing: impl Fn(i32, i32) -> Tile) -> TileSquare {
        let frame = square.encode().unwrap();
        assert_eq!(frame[2], id::AREA_TILE_CHANGE);
        TileSquare::decode(&frame[3..], existing).unwrap()
    }

    #[test]
    fn a_single_plain_tile_round_trips() {
        let square = square_of(vec![Tile::block(1)], 1, 1);
        assert_eq!(round_trip(&square), square);
    }

    #[test]
    fn air_round_trips() {
        let square = square_of(vec![Tile::AIR; 4], 2, 2);
        assert_eq!(round_trip(&square), square);
    }

    #[test]
    fn a_framed_multi_tile_object_round_trips() {
        // This is the case the packet exists for: a chest or door spanning several tiles.
        let tiles = vec![
            Tile::framed(21, 0, 0),
            Tile::framed(21, 0, 18),
            Tile::framed(21, 18, 0),
            Tile::framed(21, 18, 18),
        ];
        let square = square_of(tiles, 2, 2);
        assert_eq!(round_trip(&square), square);
    }

    #[test]
    fn walls_colours_wires_and_slopes_survive() {
        let mut tile = Tile::block(1).with_wall(300);
        tile.color = 7;
        tile.wall_color = 11;
        tile.slope = 5;
        tile.flags.set(TileFlags::WIRE_RED, true);
        tile.flags.set(TileFlags::WIRE_YELLOW, true);
        tile.flags.set(TileFlags::ACTUATED, true);
        tile.flags.set(TileFlags::INVISIBLE_WALL, true);

        let decoded = round_trip(&square_of(vec![tile], 1, 1));
        assert_eq!(decoded.tiles[0], tile);
    }

    #[test]
    fn every_liquid_survives() {
        for kind in [Liquid::Water, Liquid::Lava, Liquid::Honey, Liquid::Shimmer] {
            let tile = Tile::AIR.with_liquid(kind, 128);
            let decoded = round_trip(&square_of(vec![tile], 1, 1));
            assert_eq!(decoded.tiles[0].liquid_kind, kind);
            assert_eq!(decoded.tiles[0].liquid, 128);
        }
    }

    #[test]
    fn tiles_are_indexed_column_major() {
        // The game walks x outermost, so index 1 of a 2x3 square is the tile below the origin.
        let mut tiles = vec![Tile::AIR; 6];
        tiles[1] = Tile::block(1);
        tiles[3] = Tile::block(2);
        let square = square_of(tiles, 2, 3);

        assert_eq!(square.tile(0, 1), Some(Tile::block(1)));
        assert_eq!(square.tile(1, 0), Some(Tile::block(2)));
        assert_eq!(square.tile(2, 0), None, "outside the square");
    }

    #[test]
    fn a_size_mismatch_is_refused_rather_than_truncated() {
        let square = square_of(vec![Tile::AIR; 3], 2, 2);
        assert!(matches!(
            square.encode(),
            Err(ProtoError::OutOfRange { .. })
        ));
    }

    #[test]
    fn a_truncated_square_errors_rather_than_panicking() {
        let mut w = Writer::new();
        w.i16(0).i16(0).u8(4).u8(4).u8(0).u8(1); // claims 16 tiles, supplies a fragment
        assert!(TileSquare::decode(w.as_slice(), |_, _| Tile::AIR).is_err());
    }

    // ---------------------------------------------------------------- the merge, not overwrite

    /// The bug this file exists to fix: applying an ordinary tile square (furniture, a dug block,
    /// anything that does not touch liquid at all) over ground that already holds water must
    /// leave the water exactly as it was. A real client never sets the has-liquid bit on send
    /// (`NetMessage.cs:593`), so this is every ordinary building action a player ever takes.
    ///
    /// Before the fix, `read_tile` decoded straight into a fresh `Tile::AIR` rather than merging
    /// onto the tile already there, so this assertion failed: the water came back as `0`.
    #[test]
    fn a_square_with_no_liquid_bit_leaves_existing_liquid_intact() {
        let square = square_of(vec![Tile::block(1)], 1, 1);
        let existing = Tile::AIR.with_liquid(Liquid::Water, 200);
        let decoded = round_trip_over(&square, |_, _| existing);

        assert_eq!(decoded.tiles[0].liquid, 200, "the water must survive");
        assert_eq!(decoded.tiles[0].liquid_kind, Liquid::Water);
        assert_eq!(decoded.tiles[0].block, 1, "the block change still applies");
    }

    /// Same bug, the other resource it destroyed: paint. Colour and wall colour are only written
    /// when their bit is set (`MessageBuffer.cs:1385-1392`) — an edit that says nothing about
    /// colour must not zero it.
    #[test]
    fn a_square_with_no_colour_bits_leaves_existing_paint_intact() {
        let square = square_of(vec![Tile::block(1)], 1, 1);
        let mut existing = Tile::block(2);
        existing.color = 12;
        existing.wall_color = 9;
        let decoded = round_trip_over(&square, |_, _| existing);

        assert_eq!(decoded.tiles[0].color, 12, "block paint must survive");
        assert_eq!(decoded.tiles[0].wall_color, 9, "wall paint must survive");
    }

    /// Wall presence is *not* preserved-when-absent the way liquid and paint are: vanilla always
    /// resolves it, clearing an existing wall to none when the bit is unset
    /// (`MessageBuffer.cs:1373`, `:1427-1430`). A square that says "no wall here" must actually
    /// remove one.
    #[test]
    fn an_absent_wall_bit_clears_an_existing_wall() {
        let square = square_of(vec![Tile::block(1)], 1, 1); // no `.with_wall(..)`, so no wall bit
        let existing = Tile::block(2).with_wall(300);
        let decoded = round_trip_over(&square, |_, _| existing);

        assert_eq!(decoded.tiles[0].wall, 0, "vanilla clears the wall rather than keeping it");
    }

    /// `block`/`frame_x`/`frame_y`/`slope` are all inside vanilla's own `if (tile4.active())`
    /// guard (`MessageBuffer.cs:1393-1421`): an inactive tile in the square leaves them exactly as
    /// the existing tile had them.
    #[test]
    fn an_inactive_square_tile_leaves_the_existing_block_and_frame_untouched() {
        let square = square_of(vec![Tile::AIR], 1, 1); // active bit unset
        let existing = Tile::framed(21, 18, 0);
        let decoded = round_trip_over(&square, |_, _| existing);

        assert!(!decoded.tiles[0].is_active());
        assert_eq!(decoded.tiles[0].block, existing.block);
        assert_eq!(decoded.tiles[0].frame_x, existing.frame_x);
        assert_eq!(decoded.tiles[0].frame_y, existing.frame_y);
    }

    /// Changing a non-frame-important type resets the frame to `-1, -1` rather than leaving the
    /// old type's frame behind (`MessageBuffer.cs:1402-1406`) — the case this matters for is a
    /// dug-out block replaced by a different plain one in the same edit.
    #[test]
    fn changing_a_non_frame_important_type_resets_the_frame() {
        let square = square_of(vec![Tile::block(2)], 1, 1);
        let mut existing = Tile::block(1);
        existing.frame_x = 5;
        existing.frame_y = 5;
        let decoded = round_trip_over(&square, |_, _| existing);

        assert_eq!(decoded.tiles[0].block, 2);
        assert_eq!(decoded.tiles[0].frame_x, -1);
        assert_eq!(decoded.tiles[0].frame_y, -1);
    }

    #[test]
    fn a_large_square_round_trips() {
        let tiles: Vec<Tile> = (0..(16 * 16))
            .map(|i| {
                if i % 3 == 0 {
                    Tile::block(1)
                } else {
                    Tile::AIR
                }
            })
            .collect();
        let square = square_of(tiles, 16, 16);
        assert_eq!(round_trip(&square), square);
    }
}
