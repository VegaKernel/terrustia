//! Tile section coding — packet `10`.
//!
//! The whole payload is a raw DEFLATE stream (.NET `DeflateStream`, so no zlib or gzip wrapper),
//! and the section header is *inside* it. Older community documentation describes a leading
//! "is compressed" flag byte; that is not what 1.4.5 does. See `docs/protocol-notes.md`.

use std::io::{Read, Write};

use flate2::{Compression, read::DeflateDecoder, write::DeflateEncoder};

use crate::{
    error::{ProtoError, Result},
    id,
    reader::PacketReader,
    tile::{Liquid, Tile, TileFlags},
    tile_sets::{allows_batching, frame_important},
    writer::{PacketWriter, Writer},
};

/// A world section is a fixed 200x150 block of tiles; `SendSection` ships one per packet.
pub const SECTION_WIDTH: i32 = 200;
pub const SECTION_HEIGHT: i32 = 150;

/// The rectangle a section packet covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionBounds {
    pub x: i32,
    pub y: i32,
    pub width: i16,
    pub height: i16,
}

impl SectionBounds {
    /// The bounds of section `(section_x, section_y)` in the 200x150 grid.
    pub fn of_section(section_x: i32, section_y: i32) -> Self {
        Self {
            x: section_x * SECTION_WIDTH,
            y: section_y * SECTION_HEIGHT,
            width: SECTION_WIDTH as i16,
            height: SECTION_HEIGHT as i16,
        }
    }

    pub fn tile_count(&self) -> usize {
        self.width as usize * self.height as usize
    }
}

/// A chest announced alongside the tiles that make it up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChestInfo {
    pub id: i16,
    pub x: i16,
    pub y: i16,
    pub name: String,
}

/// A sign or tombstone announced alongside its tiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInfo {
    pub id: i16,
    pub x: i16,
    pub y: i16,
    pub text: String,
}

/// Non-tile contents of a section.
///
/// Tile entities are kept beside the world rather than in a section's trailer; the count is always
/// written as zero, which is what a world without them would send anyway.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SectionExtras {
    pub chests: Vec<ChestInfo>,
    pub signs: Vec<SignInfo>,
    /// The tile entities standing in this section.
    ///
    /// They ride the section rather than being sent one at a time because that is the only
    /// moment a client learns about the ones that were already there. Without them a world's
    /// item frames arrive empty and its pylons cannot be travelled to, however many times the
    /// section is re-sent.
    pub tile_entities: Vec<crate::tile_entity::TileEntity>,
}

/// Serialise the uncompressed section stream: header, run-length tiles, then the trailers.
pub fn write_section_stream<F>(
    out: &mut Writer,
    bounds: SectionBounds,
    extras: &SectionExtras,
    get: F,
) where
    F: Fn(i32, i32) -> Tile,
{
    out.i32(bounds.x)
        .i32(bounds.y)
        .i16(bounds.width)
        .i16(bounds.height);

    // `run` counts *additional* copies after the first, matching the game: a lone tile writes no
    // count at all and sets neither run-length flag.
    let mut pending: Option<(Tile, u16)> = None;
    for y in bounds.y..bounds.y + i32::from(bounds.height) {
        for x in bounds.x..bounds.x + i32::from(bounds.width) {
            let tile = get(x, y);
            match pending {
                Some((prev, ref mut run)) if prev == tile && allows_batching(tile.block) => {
                    *run += 1;
                }
                _ => {
                    if let Some((prev, run)) = pending.take() {
                        write_tile(out, &prev, run);
                    }
                    pending = Some((tile, 0));
                }
            }
        }
    }
    if let Some((prev, run)) = pending.take() {
        write_tile(out, &prev, run);
    }

    out.i16(extras.chests.len() as i16);
    for chest in &extras.chests {
        out.i16(chest.id)
            .i16(chest.x)
            .i16(chest.y)
            .string(&chest.name);
    }
    out.i16(extras.signs.len() as i16);
    for sign in &extras.signs {
        out.i16(sign.id).i16(sign.x).i16(sign.y).string(&sign.text);
    }
    // The section carries the *file* form of an entity — with its id, and with a logic sensor's
    // state — rather than the network form used by the sharing packet. The game writes it with
    // `TileEntity.Write`'s default argument, which is easy to miss.
    out.i16(extras.tile_entities.len() as i16);
    for entity in &extras.tile_entities {
        entity.write(out, false);
    }
}

/// Encode one tile plus its trailing run length, using the built-in frame table.
fn write_tile(out: &mut Writer, tile: &Tile, run: u16) {
    write_tile_with(out, tile, run, &frame_important)
}

/// Encode one tile, deciding frame importance with `importance`.
///
/// The `.wld` format carries its own table, and a save must be written with the table it declares
/// or the file will not read back.
pub fn write_tile_with(out: &mut Writer, tile: &Tile, run: u16, importance: &dyn Fn(u16) -> bool) {
    let (mut flags1, mut flags2, mut flags3, mut flags4) = (0u8, 0u8, 0u8, 0u8);

    // The body is built first because several of its fields decide flag bits, and the flags have
    // to precede it on the wire.
    let mut body = [0u8; 16];
    let mut len = 0usize;
    let push = |body: &mut [u8; 16], len: &mut usize, byte: u8| {
        body[*len] = byte;
        *len += 1;
    };

    if tile.is_active() {
        flags1 |= 0x02;
        push(&mut body, &mut len, tile.block as u8);
        if tile.block > 255 {
            push(&mut body, &mut len, (tile.block >> 8) as u8);
            flags1 |= 0x20;
        }
        if importance(tile.block) {
            let fx = tile.frame_x.to_le_bytes();
            let fy = tile.frame_y.to_le_bytes();
            push(&mut body, &mut len, fx[0]);
            push(&mut body, &mut len, fx[1]);
            push(&mut body, &mut len, fy[0]);
            push(&mut body, &mut len, fy[1]);
        }
        if tile.color != 0 {
            flags3 |= 0x08;
            push(&mut body, &mut len, tile.color);
        }
    }

    if tile.wall != 0 {
        flags1 |= 0x04;
        push(&mut body, &mut len, tile.wall as u8);
        if tile.wall_color != 0 {
            flags3 |= 0x10;
            push(&mut body, &mut len, tile.wall_color);
        }
    }

    if tile.liquid != 0 {
        match tile.liquid_kind {
            // Shimmer rides in the water slot with a flags3 bit to distinguish it.
            Liquid::Shimmer => {
                flags3 |= 0x80;
                flags1 |= 0x08;
            }
            Liquid::Water => flags1 |= 0x08,
            Liquid::Lava => flags1 |= 0x10,
            Liquid::Honey => flags1 |= 0x18,
        }
        push(&mut body, &mut len, tile.liquid);
    }

    if tile.flags.has(TileFlags::WIRE_RED) {
        flags2 |= 0x02;
    }
    if tile.flags.has(TileFlags::WIRE_BLUE) {
        flags2 |= 0x04;
    }
    if tile.flags.has(TileFlags::WIRE_GREEN) {
        flags2 |= 0x08;
    }
    // Half bricks encode as 1; a real slope encodes as slope + 1.
    let slope_bits = if tile.flags.has(TileFlags::HALF_BRICK) {
        1
    } else if tile.slope != 0 {
        tile.slope + 1
    } else {
        0
    };
    flags2 |= slope_bits << 4;

    if tile.flags.has(TileFlags::ACTUATOR) {
        flags3 |= 0x02;
    }
    if tile.flags.has(TileFlags::ACTUATED) {
        flags3 |= 0x04;
    }
    if tile.flags.has(TileFlags::WIRE_YELLOW) {
        flags3 |= 0x20;
    }
    if tile.wall > 255 {
        push(&mut body, &mut len, (tile.wall >> 8) as u8);
        flags3 |= 0x40;
    }

    if tile.flags.has(TileFlags::INVISIBLE_BLOCK) {
        flags4 |= 0x02;
    }
    if tile.flags.has(TileFlags::INVISIBLE_WALL) {
        flags4 |= 0x04;
    }
    if tile.flags.has(TileFlags::FULLBRIGHT_BLOCK) {
        flags4 |= 0x08;
    }
    if tile.flags.has(TileFlags::FULLBRIGHT_WALL) {
        flags4 |= 0x10;
    }

    if run > 0 {
        push(&mut body, &mut len, run as u8);
        if run > 255 {
            push(&mut body, &mut len, (run >> 8) as u8);
            flags1 |= 0x80;
        } else {
            flags1 |= 0x40;
        }
    }

    // Each flag byte's bit 0 means "another follows", so set them from the tail inwards.
    if flags4 != 0 {
        flags3 |= 0x01;
    }
    if flags3 != 0 {
        flags2 |= 0x01;
    }
    if flags2 != 0 {
        flags1 |= 0x01;
    }

    out.u8(flags1);
    if flags2 != 0 {
        out.u8(flags2);
    }
    if flags3 != 0 {
        out.u8(flags3);
    }
    if flags4 != 0 {
        out.u8(flags4);
    }
    out.bytes(&body[..len]);
}

/// Build a complete `TileSection` frame: deflate the stream and wrap it in a packet header.
pub fn encode_section_packet<F>(
    bounds: SectionBounds,
    extras: &SectionExtras,
    get: F,
) -> Result<Vec<u8>>
where
    F: Fn(i32, i32) -> Tile,
{
    let mut stream = Writer::with_capacity(bounds.tile_count() / 2 + 64);
    write_section_stream(&mut stream, bounds, extras, get);

    // Best, not default. A section is encoded once and then cached, so the extra work is paid a
    // single time per section per world and the smaller result is paid out on every join for the
    // life of the server. Sections are far and away the largest thing this server sends — about
    // 45% of a session's bytes — so a few per cent off them is worth more than it looks.
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(stream.as_slice())
        .and_then(|()| encoder.finish())
        .map_err(|e| ProtoError::Deflate(e.to_string()))
        .and_then(|compressed| {
            let mut packet = PacketWriter::new(id::TILE_SECTION);
            packet.bytes(&compressed);
            packet.finish()
        })
}

/// Decode a section stream back into tiles.
///
/// This mirrors the client's `DecompressTileBlock_Inner`, so a round-trip through
/// [`write_section_stream`] and back exercises the same rules the real client applies rather than
/// just proving our encoder self-consistent.
pub fn decode_section_stream(stream: &[u8]) -> Result<(SectionBounds, Vec<Tile>, SectionExtras)> {
    let mut r = PacketReader::new(stream);
    let bounds = SectionBounds {
        x: r.i32()?,
        y: r.i32()?,
        width: r.i16()?,
        height: r.i16()?,
    };
    // A real section is always exactly `SECTION_WIDTH` x `SECTION_HEIGHT` — `SectionBounds::of_section`
    // never constructs anything else, and nothing in this codebase sends a partial one. Bounding
    // `tile_count()` to that before it drives an allocation, not just checking the sign, is load
    // bearing: found by fuzzing, not review — 8 bytes claiming a 31232x12815 section (a real,
    // structurally-valid-looking header, just absurd numbers) drove `Vec::with_capacity` to try
    // and reserve room for ~400 million `Tile`s, aborting the process on the allocation itself
    // before a single byte of tile data was ever read. This is exactly the "decode path cannot
    // panic or over-allocate" guarantee this project already claims elsewhere; it was not true of
    // this decoder until this bound existed.
    if bounds.width < 0
        || bounds.height < 0
        || bounds.tile_count() > (SECTION_WIDTH * SECTION_HEIGHT) as usize
    {
        return Err(ProtoError::OutOfRange {
            field: "section size",
            value: i64::from(bounds.width) * i64::from(bounds.height),
        });
    }

    let mut tiles = Vec::with_capacity(bounds.tile_count());
    while tiles.len() < bounds.tile_count() {
        let (tile, run) = read_tile(&mut r)?;
        // `run` is the number of *extra* copies, so a tile always contributes at least one.
        let total = usize::from(run) + 1;
        if tiles.len() + total > bounds.tile_count() {
            return Err(ProtoError::OutOfRange {
                field: "run length",
                value: run as i64,
            });
        }
        tiles.extend(std::iter::repeat_n(tile, total));
    }

    let mut extras = SectionExtras::default();
    for _ in 0..r.i16()? {
        extras.chests.push(ChestInfo {
            id: r.i16()?,
            x: r.i16()?,
            y: r.i16()?,
            name: r.string()?,
        });
    }
    for _ in 0..r.i16()? {
        extras.signs.push(SignInfo {
            id: r.i16()?,
            x: r.i16()?,
            y: r.i16()?,
            text: r.string()?,
        });
    }
    let entities = r.i16()?;
    if entities < 0 {
        return Err(ProtoError::OutOfRange {
            field: "tile entity count",
            value: i64::from(entities),
        });
    }
    for _ in 0..entities {
        extras
            .tile_entities
            .push(crate::tile_entity::TileEntity::read(&mut r, false)?);
    }

    Ok((bounds, tiles, extras))
}

fn read_tile(r: &mut PacketReader<'_>) -> Result<(Tile, u16)> {
    read_tile_with(r, &frame_important)
}

/// Decode one tile and its run length, deciding frame importance with `importance`.
///
/// The `.wld` file format encodes tiles exactly as a network section does, but carries its own
/// frame-importance table so that an old save still loads after the table changes. Sharing this
/// function means the two paths cannot drift apart.
pub fn read_tile_with(
    r: &mut PacketReader<'_>,
    importance: &dyn Fn(u16) -> bool,
) -> Result<(Tile, u16)> {
    let flags1 = r.u8()?;
    let flags2 = if flags1 & 0x01 != 0 { r.u8()? } else { 0 };
    let flags3 = if flags2 & 0x01 != 0 { r.u8()? } else { 0 };
    let flags4 = if flags3 & 0x01 != 0 { r.u8()? } else { 0 };

    let mut tile = Tile::AIR;

    if flags1 & 0x02 != 0 {
        tile.flags.set(TileFlags::ACTIVE, true);
        let low = u16::from(r.u8()?);
        tile.block = if flags1 & 0x20 != 0 {
            (u16::from(r.u8()?) << 8) | low
        } else {
            low
        };
        if importance(tile.block) {
            tile.frame_x = r.i16()?;
            tile.frame_y = r.i16()?;
        }
        if flags3 & 0x08 != 0 {
            tile.color = r.u8()?;
        }
    }

    if flags1 & 0x04 != 0 {
        tile.wall = u16::from(r.u8()?);
        if flags3 & 0x10 != 0 {
            tile.wall_color = r.u8()?;
        }
    }

    let liquid_bits = (flags1 & 0x18) >> 3;
    if liquid_bits != 0 {
        tile.liquid = r.u8()?;
        tile.liquid_kind = if flags3 & 0x80 != 0 {
            Liquid::Shimmer
        } else {
            match liquid_bits {
                2 => Liquid::Lava,
                3 => Liquid::Honey,
                _ => Liquid::Water,
            }
        };
    }

    tile.flags.set(TileFlags::WIRE_RED, flags2 & 0x02 != 0);
    tile.flags.set(TileFlags::WIRE_BLUE, flags2 & 0x04 != 0);
    tile.flags.set(TileFlags::WIRE_GREEN, flags2 & 0x08 != 0);
    match (flags2 & 0x70) >> 4 {
        0 => {}
        1 => tile.flags.set(TileFlags::HALF_BRICK, true),
        n => tile.slope = n - 1,
    }

    tile.flags.set(TileFlags::ACTUATOR, flags3 & 0x02 != 0);
    tile.flags.set(TileFlags::ACTUATED, flags3 & 0x04 != 0);
    tile.flags.set(TileFlags::WIRE_YELLOW, flags3 & 0x20 != 0);
    if flags3 & 0x40 != 0 {
        tile.wall |= u16::from(r.u8()?) << 8;
    }

    tile.flags
        .set(TileFlags::INVISIBLE_BLOCK, flags4 & 0x02 != 0);
    tile.flags
        .set(TileFlags::INVISIBLE_WALL, flags4 & 0x04 != 0);
    tile.flags
        .set(TileFlags::FULLBRIGHT_BLOCK, flags4 & 0x08 != 0);
    tile.flags
        .set(TileFlags::FULLBRIGHT_WALL, flags4 & 0x10 != 0);

    let run = match (flags1 & 0xC0) >> 6 {
        0 => 0,
        1 => u16::from(r.u8()?),
        _ => r.i16()? as u16,
    };

    Ok((tile, run))
}

/// Inflate a `TileSection` payload back into its uncompressed stream.
pub fn inflate_section_payload(payload: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    DeflateDecoder::new(payload)
        .read_to_end(&mut out)
        .map_err(|e| ProtoError::Deflate(e.to_string()))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_FRAME_LEN;

    /// Encode a section from a closure and decode it straight back.
    fn round_trip<F: Fn(i32, i32) -> Tile>(bounds: SectionBounds, get: F) -> Vec<Tile> {
        let mut stream = Writer::new();
        write_section_stream(&mut stream, bounds, &SectionExtras::default(), &get);
        let (decoded_bounds, tiles, extras) = decode_section_stream(stream.as_slice()).unwrap();
        assert_eq!(decoded_bounds, bounds);
        assert_eq!(extras, SectionExtras::default());
        assert_eq!(tiles.len(), bounds.tile_count());

        // Every tile must come back exactly as it went in.
        for (i, tile) in tiles.iter().enumerate() {
            let x = bounds.x + (i % bounds.width as usize) as i32;
            let y = bounds.y + (i / bounds.width as usize) as i32;
            assert_eq!(*tile, get(x, y), "tile at ({x}, {y}) changed");
        }
        tiles
    }

    fn small() -> SectionBounds {
        SectionBounds {
            x: 0,
            y: 0,
            width: 8,
            height: 4,
        }
    }

    #[test]
    fn all_air_round_trips() {
        round_trip(small(), |_, _| Tile::AIR);
    }

    #[test]
    fn uniform_stone_round_trips() {
        round_trip(small(), |_, _| Tile::block(1));
    }

    #[test]
    fn alternating_tiles_defeat_batching_and_still_round_trip() {
        // Worst case for the encoder: no two adjacent tiles are equal.
        round_trip(small(), |x, y| {
            if (x + y) % 2 == 0 {
                Tile::block(1)
            } else {
                Tile::AIR
            }
        });
    }

    #[test]
    fn two_byte_tile_types_round_trip() {
        // Anything above 255 sets the flags1 0x20 bit and writes a second byte.
        round_trip(small(), |_, _| Tile::block(400));
    }

    #[test]
    fn two_byte_wall_types_round_trip() {
        // Walls above 255 put their high byte after the liquid, not next to the low byte.
        round_trip(small(), |_, _| Tile::block(1).with_wall(300));
    }

    #[test]
    fn every_liquid_round_trips() {
        for kind in [Liquid::Water, Liquid::Lava, Liquid::Honey, Liquid::Shimmer] {
            let tiles = round_trip(small(), |_, _| Tile::AIR.with_liquid(kind, 255));
            assert_eq!(tiles[0].liquid_kind, kind);
            assert_eq!(tiles[0].liquid, 255);
        }
    }

    #[test]
    fn frame_important_tiles_carry_their_frames() {
        let tiles = round_trip(small(), |x, _| Tile::framed(21, x as i16 * 18, 0));
        assert_eq!(tiles[3].frame_x, 54);
    }

    #[test]
    fn colours_wires_and_slopes_round_trip() {
        let tiles = round_trip(small(), |_, _| {
            let mut t = Tile::block(1).with_wall(2);
            t.color = 5;
            t.wall_color = 9;
            t.slope = 3;
            t.flags.set(TileFlags::WIRE_RED, true);
            t.flags.set(TileFlags::WIRE_YELLOW, true);
            t.flags.set(TileFlags::ACTUATOR, true);
            t.flags.set(TileFlags::FULLBRIGHT_WALL, true);
            t
        });
        let t = tiles[0];
        assert_eq!((t.color, t.wall_color, t.slope), (5, 9, 3));
        assert!(t.flags.has(TileFlags::WIRE_RED));
        assert!(t.flags.has(TileFlags::WIRE_YELLOW));
        assert!(t.flags.has(TileFlags::ACTUATOR));
        assert!(t.flags.has(TileFlags::FULLBRIGHT_WALL));
    }

    #[test]
    fn half_brick_and_slope_share_the_same_bits() {
        let tiles = round_trip(small(), |_, _| {
            let mut t = Tile::block(1);
            t.flags.set(TileFlags::HALF_BRICK, true);
            t
        });
        assert!(tiles[0].flags.has(TileFlags::HALF_BRICK));
        assert_eq!(tiles[0].slope, 0);
    }

    #[test]
    fn runs_longer_than_255_use_the_two_byte_count() {
        // A full 200-wide row repeated is over 255 identical tiles, forcing the wide count.
        let bounds = SectionBounds {
            x: 0,
            y: 0,
            width: 200,
            height: 4,
        };
        let mut stream = Writer::new();
        write_section_stream(&mut stream, bounds, &SectionExtras::default(), |_, _| {
            Tile::block(1)
        });

        // One tile, a two-byte run count, then three empty trailers.
        assert_eq!(
            stream.as_slice()[12] & 0xC0,
            0x80,
            "expected the wide run flag"
        );
        round_trip(bounds, |_, _| Tile::block(1));
    }

    #[test]
    fn a_run_of_exactly_256_is_wide_and_255_is_narrow() {
        // The count is "extras after the first", so 256 tiles means a count of 255 (narrow) and
        // 257 tiles means 256 (wide). This boundary is easy to get off by one.
        for (count, expect_wide) in [(256usize, false), (257, true)] {
            let bounds = SectionBounds {
                x: 0,
                y: 0,
                width: count as i16,
                height: 1,
            };
            let mut stream = Writer::new();
            write_section_stream(&mut stream, bounds, &SectionExtras::default(), |_, _| {
                Tile::block(1)
            });
            let flags = stream.as_slice()[12];
            assert_eq!(
                flags & 0xC0,
                if expect_wide { 0x80 } else { 0x40 },
                "{count} tiles"
            );
            round_trip(bounds, |_, _| Tile::block(1));
        }
    }

    #[test]
    fn tiles_that_forbid_batching_are_never_merged() {
        // Type 520 is one of the four exceptions; each one must appear separately.
        let bounds = SectionBounds {
            x: 0,
            y: 0,
            width: 4,
            height: 1,
        };
        let mut stream = Writer::new();
        write_section_stream(&mut stream, bounds, &SectionExtras::default(), |_, _| {
            Tile::framed(520, 0, 0)
        });
        // Header is 12 bytes; four unmerged tiles each set no run flag.
        let body = &stream.as_slice()[12..];
        assert_eq!(
            body[0] & 0xC0,
            0,
            "batching-exempt tile should carry no run"
        );
        round_trip(bounds, |_, _| Tile::framed(520, 0, 0));
    }

    #[test]
    fn chests_and_signs_survive_the_trailers() {
        let bounds = small();
        let extras = SectionExtras {
            chests: vec![ChestInfo {
                id: 3,
                x: 10,
                y: 20,
                name: "Loot".into(),
            }],
            signs: vec![SignInfo {
                id: 1,
                x: 5,
                y: 6,
                text: "hello \u{1F600}".into(),
            }],
            tile_entities: vec![{
                let mut frame = crate::tile_entity::TileEntity::new(
                    9,
                    crate::tile_entity::EntityKind::ItemFrame,
                    7,
                    8,
                );
                frame.data =
                    crate::tile_entity::EntityData::Held(crate::ItemStack::new(3507, 1, 0));
                frame
            }],
        };
        let mut stream = Writer::new();
        write_section_stream(&mut stream, bounds, &extras, |_, _| Tile::AIR);
        let (_, _, decoded) = decode_section_stream(stream.as_slice()).unwrap();
        assert_eq!(decoded, extras);
    }

    #[test]
    fn the_packet_is_a_bare_deflate_stream_with_no_flag_byte() {
        let bounds = small();
        let frame = encode_section_packet(bounds, &SectionExtras::default(), |_, _| Tile::block(1))
            .unwrap();

        assert_eq!(frame[2], id::TILE_SECTION);
        assert_eq!(
            u16::from_le_bytes([frame[0], frame[1]]) as usize,
            frame.len()
        );

        // Inflating from byte 3 must work; if a flag byte preceded the stream it would not.
        let stream = inflate_section_payload(&frame[3..]).unwrap();
        let (decoded_bounds, tiles, _) = decode_section_stream(&stream).unwrap();
        assert_eq!(decoded_bounds, bounds);
        assert!(tiles.iter().all(|t| *t == Tile::block(1)));
    }

    #[test]
    fn a_full_section_encodes_and_survives_the_frame_limit() {
        // A realistic mixed section: sky, dirt, stone with an ore vein. This must both encode and
        // fit inside a single 65535-byte frame, which is the reason sections are 200x150.
        let bounds = SectionBounds::of_section(3, 2);
        let get = |x: i32, y: i32| {
            if y < 400 {
                Tile::AIR
            } else if y < 420 {
                Tile::block(0).with_wall(2)
            } else if (x * 7 + y * 13) % 97 == 0 {
                Tile::block(7)
            } else {
                Tile::block(1)
            }
        };
        let frame = encode_section_packet(bounds, &SectionExtras::default(), get).unwrap();
        assert!(
            frame.len() <= MAX_FRAME_LEN,
            "section frame was {} bytes",
            frame.len()
        );

        let stream = inflate_section_payload(&frame[3..]).unwrap();
        let (_, tiles, _) = decode_section_stream(&stream).unwrap();
        assert_eq!(tiles.len(), bounds.tile_count());
        for (i, tile) in tiles.iter().enumerate() {
            let x = bounds.x + (i % bounds.width as usize) as i32;
            let y = bounds.y + (i / bounds.width as usize) as i32;
            assert_eq!(*tile, get(x, y), "tile at ({x}, {y})");
        }
    }

    #[test]
    fn a_truncated_stream_is_an_error_not_a_panic() {
        let bounds = small();
        let mut stream = Writer::new();
        write_section_stream(&mut stream, bounds, &SectionExtras::default(), |_, _| {
            Tile::block(1)
        });
        let bytes = stream.into_bytes();
        for cut in [0, 4, 8, 11, 13] {
            assert!(
                decode_section_stream(&bytes[..cut]).is_err(),
                "cut at {cut}"
            );
        }
    }

    #[test]
    fn a_run_overflowing_the_section_is_rejected() {
        // A hostile stream claiming a run far longer than the section must not allocate wildly.
        let mut stream = Writer::new();
        stream.i32(0).i32(0).i16(2).i16(2);
        stream.u8(0x02 | 0x80).u8(1).u8(0xFF).u8(0xFF); // active stone, run of 65535
        assert!(matches!(
            decode_section_stream(stream.as_slice()),
            Err(ProtoError::OutOfRange { .. })
        ));
    }

    /// Found by `cargo fuzz run section_stream` within its first 90 seconds, not by review: a
    /// header claiming a 31232x12815 section — a structurally plausible header, just absurd
    /// numbers — drove `Vec::with_capacity` to try to reserve room for ~400 million `Tile`s and
    /// aborted the process on the allocation itself, before a single byte of tile data was ever
    /// read. Only the *sign* of width/height was checked before this; nothing bounded their size,
    /// even though every section this project ever constructs is exactly `SECTION_WIDTH` x
    /// `SECTION_HEIGHT`. This is the exact fuzzer-found input, minimized to just the header.
    #[test]
    fn an_oversized_section_header_is_rejected_before_it_allocates() {
        let mut stream = Writer::new();
        stream.i32(983060).i32(76672).i16(31232).i16(12815);
        assert!(
            matches!(
                decode_section_stream(stream.as_slice()),
                Err(ProtoError::OutOfRange { .. })
            ),
            "an oversized section header must be rejected before Vec::with_capacity ever runs"
        );
    }
}
