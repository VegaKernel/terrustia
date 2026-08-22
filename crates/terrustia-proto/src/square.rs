//! Packet `20`, `AreaTileChange` — a small rectangle of tiles pushed as a unit.
//!
//! Clients send this whenever a change spans more than one tile: placing furniture, opening a
//! door, growing a tree. Applying it is what lets a server stay in step with multi-tile operations
//! without reimplementing the game's placement and framing rules.
//!
//! The encoding is denser than a section's and completely different: three flag bytes per tile, no
//! run-length, and tiles walked **column by column** within the rectangle.

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

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        let x = r.i16()?;
        let y = r.i16()?;
        let width = r.u8()?;
        let height = r.u8()?;
        let change_type = r.u8()?;

        let count = usize::from(width) * usize::from(height);
        let mut tiles = Vec::with_capacity(count);
        for _ in 0..count {
            tiles.push(read_tile(&mut r)?);
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

fn read_tile(r: &mut PacketReader<'_>) -> Result<Tile> {
    let flags1 = r.u8()?;
    let flags2 = r.u8()?;
    let flags3 = r.u8()?;

    let mut tile = Tile::AIR;
    let active = flags1 & 0x01 != 0;
    let has_wall = flags1 & 0x04 != 0;
    let has_liquid = flags1 & 0x08 != 0;

    tile.flags.set(TileFlags::ACTIVE, active);
    tile.flags.set(TileFlags::WIRE_RED, flags1 & 0x10 != 0);
    tile.flags.set(TileFlags::HALF_BRICK, flags1 & 0x20 != 0);
    tile.flags.set(TileFlags::ACTUATOR, flags1 & 0x40 != 0);
    tile.flags.set(TileFlags::ACTUATED, flags1 & 0x80 != 0);
    tile.flags.set(TileFlags::WIRE_BLUE, flags2 & 0x01 != 0);
    tile.flags.set(TileFlags::WIRE_GREEN, flags2 & 0x02 != 0);
    tile.flags.set(TileFlags::WIRE_YELLOW, flags2 & 0x80 != 0);

    if flags2 & 0x04 != 0 {
        tile.color = r.u8()?;
    }
    if flags2 & 0x08 != 0 {
        tile.wall_color = r.u8()?;
    }

    if active {
        // Unlike a section, the type is always two bytes here.
        tile.block = r.u16()?;
        if frame_important(tile.block) {
            tile.frame_x = r.i16()?;
            tile.frame_y = r.i16()?;
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

    fn round_trip(square: &TileSquare) -> TileSquare {
        let frame = square.encode().unwrap();
        assert_eq!(frame[2], id::AREA_TILE_CHANGE);
        TileSquare::decode(&frame[3..]).unwrap()
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
        assert!(TileSquare::decode(w.as_slice()).is_err());
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
