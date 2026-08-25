//! How a world's five million tiles are actually stored.
//!
//! [`Tile`] is a convenient value type — ten named fields, copied freely, sixteen bytes with
//! padding. That is fine on the stack and ruinous in an array: a small world is 4200x1200, so every
//! byte of the struct costs five megabytes, and the full sixteen come to **80.6 MB** before the
//! server has done anything at all.
//!
//! Most of those bytes are paid by tiles that have no use for them. Measured on a real world:
//!
//! | field | size | tiles that need it |
//! |---|---|---|
//! | `frame_x` / `frame_y` | 4 bytes, 20.2 MB | **1.87%** |
//! | `color` / `wall_color` | 2 bytes, 10.1 MB | **0.00%** |
//! | `slope` | 1 byte, 5.0 MB | 1.17%, and it only needs three bits |
//! | `liquid_kind` | 1 byte, 5.0 MB | only where there is liquid, and it needs two bits |
//!
//! So the array holds [`PackedTile`] — **eight bytes**, everything every tile genuinely needs — and
//! the two rare pairs live in side tables keyed by position. Frames cost 1.1 MB that way against
//! 20.2 MB inline; paint costs nothing at all on a world nobody has painted.
//!
//! The tile array halves, from 80.6 MB to 40.3 MB.
//!
//! **The `Tile` API does not change.** `World::tile` reassembles one on the way out and
//! `World::set_tile` takes one apart on the way in, so the hundred-odd places that read
//! `tile.frame_x` or `tile.slope` are untouched. The side tables are only consulted when the packed
//! tile says there is something in them, which is why the ninety-eight per cent of tiles that are
//! dirt and stone pay a bit test rather than a hash lookup.

use std::collections::HashMap;

use terrustia_proto::{Liquid, Tile, TileFlags};

/// One tile as the world array stores it: eight bytes, no padding.
///
/// Field order is chosen so the struct packs exactly — two shorts, then a short of flags, then two
/// bytes — rather than relying on the compiler to find the arrangement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PackedTile {
    block: u16,
    wall: u16,
    /// [`TileFlags`] verbatim, so nothing here has to know what the bits mean.
    flags: u16,
    liquid: u8,
    /// Everything that fits in a handful of bits: the slope, which liquid, and whether this tile
    /// has an entry in each side table.
    ///
    /// The two "has an entry" bits live here rather than in [`TileFlags`] on purpose. `TileFlags`
    /// is part of `Tile` and travels to clients and into save files; a storage detail that leaked
    /// into either would be a wire format that depended on how this server happened to keep its
    /// memory.
    extra: u8,
}

impl PackedTile {
    const SLOPE: u8 = 0b0000_0111;
    const LIQUID_KIND: u8 = 0b0001_1000;
    const LIQUID_KIND_SHIFT: u8 = 3;
    const HAS_FRAME: u8 = 0b0010_0000;
    const HAS_PAINT: u8 = 0b0100_0000;
}

/// A world's tiles, stored compactly.
///
/// Kept as its own type rather than three fields on `World` so the invariant that matters — a side
/// table holds an entry exactly when the packed tile says it does — is enforced in one place by two
/// methods, instead of being a rule every caller has to remember.
#[derive(Debug, Clone)]
pub struct TileStore {
    width: i32,
    tiles: Vec<PackedTile>,
    /// `frameX` and `frameY` for the tiles that have them — chests, doors, furniture, plants.
    frames: HashMap<u32, (i16, i16)>,
    /// Paint on the block and on the wall, for the tiles that have any.
    paint: HashMap<u32, (u8, u8)>,
}

impl TileStore {
    /// Overwrite this store from another, reusing the allocation already here.
    ///
    /// `clone()` asks the allocator for a fresh forty-megabyte mapping and then faults in every
    /// page of it as it writes. Copying into a buffer we already own touches pages that are
    /// already mapped, which is the difference between a memcpy and a memcpy plus ten thousand
    /// page faults. `Vec::clone_from` keeps the existing capacity when it is large enough, which
    /// for a fixed-size world it always is.
    /// Copy one rectangle of tiles from another store of the same size.
    ///
    /// Row by row, because the array is row-major: a rectangle is a run of contiguous slices, one
    /// per row, and `copy_from_slice` on each is a memcpy. Used to bring a snapshot buffer up to
    /// date without recopying the whole world.
    pub fn copy_rect_from(&mut self, other: &Self, x0: i32, y0: i32, x1: i32, y1: i32) {
        debug_assert_eq!(self.width, other.width, "stores must be the same shape");
        let width = self.width.max(0) as usize;
        if width == 0 {
            return;
        }
        let rows = self.tiles.len() / width;
        let (x0, x1) = (x0.max(0) as usize, (x1.max(0) as usize).min(width));
        let (y0, y1) = (y0.max(0) as usize, (y1.max(0) as usize).min(rows));
        if x0 >= x1 {
            return;
        }
        for y in y0..y1 {
            let from = y * width + x0;
            let to = y * width + x1;
            self.tiles[from..to].copy_from_slice(&other.tiles[from..to]);
        }
    }

    /// Replace the side tables wholesale.
    ///
    /// The frame and paint tables are keyed by tile index, so there is no cheap way to update just
    /// one rectangle of them. They are small next to the tile array — thousands of entries against
    /// millions of tiles — so they are simply copied every time.
    pub fn copy_side_tables_from(&mut self, other: &Self) {
        self.frames.clone_from(&other.frames);
        self.paint.clone_from(&other.paint);
    }

    pub fn copy_from(&mut self, other: &Self) {
        self.width = other.width;
        self.tiles.clone_from(&other.tiles);
        self.frames.clone_from(&other.frames);
        self.paint.clone_from(&other.paint);
    }

    pub fn new(width: i32, height: i32) -> Self {
        let count = (width.max(0) as usize) * (height.max(0) as usize);
        Self {
            width,
            // Air is all zeroes except its frames, which are -1 and live in no table: a tile with
            // no frame entry reads back as -1, which is exactly what air wants.
            tiles: vec![PackedTile::default(); count],
            frames: HashMap::new(),
            paint: HashMap::new(),
        }
    }

    fn index(&self, x: i32, y: i32) -> u32 {
        (y * self.width + x) as u32
    }

    /// Reassemble the tile at a position.
    ///
    /// Bounds are the caller's business; `World::tile` checks them and returns air outside.
    pub fn get(&self, x: i32, y: i32) -> Tile {
        let at = self.index(x, y);
        let packed = self.tiles[at as usize];

        // Absent from the table means no frame, and no frame means -1: the value the game uses for
        // "this tile has no frame", and what `Tile::AIR` carries.
        let (frame_x, frame_y) = if packed.extra & PackedTile::HAS_FRAME != 0 {
            self.frames.get(&at).copied().unwrap_or((-1, -1))
        } else {
            (-1, -1)
        };
        let (color, wall_color) = if packed.extra & PackedTile::HAS_PAINT != 0 {
            self.paint.get(&at).copied().unwrap_or((0, 0))
        } else {
            (0, 0)
        };

        Tile {
            block: packed.block,
            wall: packed.wall,
            frame_x,
            frame_y,
            liquid: packed.liquid,
            liquid_kind: match (packed.extra & PackedTile::LIQUID_KIND)
                >> PackedTile::LIQUID_KIND_SHIFT
            {
                1 => Liquid::Lava,
                2 => Liquid::Honey,
                3 => Liquid::Shimmer,
                _ => Liquid::Water,
            },
            color,
            wall_color,
            slope: packed.extra & PackedTile::SLOPE,
            flags: TileFlags(packed.flags),
        }
    }

    /// Take a tile apart into the array and, where it has anything to say, the side tables.
    ///
    /// A tile that no longer has a frame or paint has its entry **removed**, not left behind. That
    /// is the whole cost model: a table that only grows would eventually hold an entry for every
    /// position that had ever been a door, and be worse than the four bytes it replaced.
    pub fn set(&mut self, x: i32, y: i32, tile: Tile) {
        let at = self.index(x, y);
        // What was here before, so a side table is only touched when it actually has to be.
        //
        // This matters far more than it looks. Loading a world calls this five million times, and
        // clearing an entry that was never there still costs a hash of the key. Doing it
        // unconditionally put ten million pointless hash operations into world loading and took it
        // from a fifth of a second to three and a half; the array read that avoids them is free by
        // comparison, because that cache line is about to be written anyway.
        let previous = self.tiles[at as usize].extra;

        let has_frame = tile.frame_x != -1 || tile.frame_y != -1;
        if has_frame {
            self.frames.insert(at, (tile.frame_x, tile.frame_y));
        } else if previous & PackedTile::HAS_FRAME != 0 {
            self.frames.remove(&at);
        }

        let has_paint = tile.color != 0 || tile.wall_color != 0;
        if has_paint {
            self.paint.insert(at, (tile.color, tile.wall_color));
        } else if previous & PackedTile::HAS_PAINT != 0 {
            self.paint.remove(&at);
        }

        let liquid_kind = match tile.liquid_kind {
            Liquid::Water => 0u8,
            Liquid::Lava => 1,
            Liquid::Honey => 2,
            Liquid::Shimmer => 3,
        };
        let mut extra = (tile.slope & PackedTile::SLOPE)
            | (liquid_kind << PackedTile::LIQUID_KIND_SHIFT);
        if has_frame {
            extra |= PackedTile::HAS_FRAME;
        }
        if has_paint {
            extra |= PackedTile::HAS_PAINT;
        }

        self.tiles[at as usize] = PackedTile {
            block: tile.block,
            wall: tile.wall,
            flags: tile.flags.0,
            liquid: tile.liquid,
            extra,
        };
    }

    /// What the tile array and its side tables cost, in bytes, for reporting.
    pub fn footprint(&self) -> (usize, usize, usize) {
        (
            self.tiles.len() * size_of::<PackedTile>(),
            // A `HashMap` entry costs its key and value plus control overhead; 1.4x is the usual
            // figure for the load factors this ends up at.
            (self.frames.len() * (4 + 4) * 7) / 5,
            (self.paint.len() * (4 + 2) * 7) / 5,
        )
    }

    pub fn framed_tiles(&self) -> usize {
        self.frames.len()
    }

    pub fn painted_tiles(&self) -> usize {
        self.paint.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: eight bytes, not sixteen.
    #[test]
    fn a_packed_tile_is_eight_bytes() {
        assert_eq!(size_of::<PackedTile>(), 8);
        // And the thing it replaces, for the contrast the module exists to make.
        assert_eq!(size_of::<Tile>(), 16);
    }

    #[test]
    fn every_field_survives_being_taken_apart_and_put_back() {
        let mut store = TileStore::new(16, 16);
        let tile = Tile {
            block: 21,
            wall: 155,
            frame_x: 36,
            frame_y: 18,
            liquid: 200,
            liquid_kind: Liquid::Honey,
            color: 13,
            wall_color: 7,
            slope: 3,
            flags: TileFlags(0b1010_1010_1010),
        };
        store.set(3, 4, tile);
        assert_eq!(store.get(3, 4), tile);
    }

    #[test]
    fn each_liquid_survives() {
        let mut store = TileStore::new(8, 8);
        for kind in [Liquid::Water, Liquid::Lava, Liquid::Honey, Liquid::Shimmer] {
            let tile = Tile {
                liquid: 255,
                liquid_kind: kind,
                ..Tile::AIR
            };
            store.set(1, 1, tile);
            assert_eq!(store.get(1, 1).liquid_kind, kind, "{kind:?}");
        }
    }

    #[test]
    fn air_reads_back_as_air() {
        let store = TileStore::new(8, 8);
        assert_eq!(store.get(0, 0), Tile::AIR, "an untouched tile is air");
        assert_eq!(store.get(7, 7).frame_x, -1, "with no frame");
    }

    /// A side table must shrink as well as grow.
    ///
    /// It is only cheaper than the bytes it replaces while it stays small. One that only ever grew
    /// would end up holding an entry for every position that had ever been a door, which is worse
    /// than the four bytes per tile it was meant to save.
    #[test]
    fn clearing_a_tile_gives_its_side_entries_back() {
        let mut store = TileStore::new(8, 8);
        store.set(
            2,
            2,
            Tile {
                block: 21,
                frame_x: 0,
                frame_y: 0,
                color: 5,
                ..Tile::AIR
            },
        );
        assert_eq!(store.framed_tiles(), 1);
        assert_eq!(store.painted_tiles(), 1);

        store.set(2, 2, Tile::AIR);
        assert_eq!(store.framed_tiles(), 0, "the frame entry should be gone");
        assert_eq!(store.painted_tiles(), 0, "and the paint entry with it");
        assert_eq!(store.get(2, 2), Tile::AIR);
    }

    /// Two tiles must not share a side entry.
    #[test]
    fn side_entries_are_per_position() {
        let mut store = TileStore::new(8, 8);
        store.set(
            1,
            0,
            Tile {
                block: 10,
                frame_x: 100,
                frame_y: 200,
                ..Tile::AIR
            },
        );
        store.set(
            2,
            0,
            Tile {
                block: 10,
                frame_x: 300,
                frame_y: 400,
                ..Tile::AIR
            },
        );
        assert_eq!((store.get(1, 0).frame_x, store.get(1, 0).frame_y), (100, 200));
        assert_eq!((store.get(2, 0).frame_x, store.get(2, 0).frame_y), (300, 400));
    }
}
