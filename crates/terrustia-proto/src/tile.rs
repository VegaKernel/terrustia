use crate::tile_sets::frame_important;

/// Which liquid occupies a tile. Only meaningful when [`Tile::liquid`] is non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Liquid {
    #[default]
    Water,
    Lava,
    Honey,
    Shimmer,
}

impl Liquid {
    /// The number the game calls `Tile.liquidType()`.
    ///
    /// Spelled out rather than taken from the variant order, because the *other* place a liquid
    /// kind goes on the wire — the section stream — encodes it as bit patterns instead
    /// (`0x10` lava, `0x18` honey), so there is nothing about this file that makes the plain
    /// ordinal obviously right. It is what net module 0 carries.
    pub fn as_type_byte(self) -> u8 {
        match self {
            Self::Water => 0,
            Self::Lava => 1,
            Self::Honey => 2,
            Self::Shimmer => 3,
        }
    }
}

/// Boolean tile attributes, packed to keep a whole world in a reasonable amount of memory.
///
/// A small world is 4200x1200 tiles, so every byte here costs about 5 MB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TileFlags(pub u16);

impl TileFlags {
    pub const ACTIVE: u16 = 1 << 0;
    pub const HALF_BRICK: u16 = 1 << 1;
    pub const ACTUATOR: u16 = 1 << 2;
    pub const ACTUATED: u16 = 1 << 3;
    pub const WIRE_RED: u16 = 1 << 4;
    pub const WIRE_BLUE: u16 = 1 << 5;
    pub const WIRE_GREEN: u16 = 1 << 6;
    pub const WIRE_YELLOW: u16 = 1 << 7;
    pub const INVISIBLE_BLOCK: u16 = 1 << 8;
    pub const INVISIBLE_WALL: u16 = 1 << 9;
    pub const FULLBRIGHT_BLOCK: u16 = 1 << 10;
    pub const FULLBRIGHT_WALL: u16 = 1 << 11;

    pub const fn has(self, bit: u16) -> bool {
        self.0 & bit != 0
    }

    pub fn set(&mut self, bit: u16, on: bool) {
        if on {
            self.0 |= bit;
        } else {
            self.0 &= !bit;
        }
    }
}

/// One tile of the world.
///
/// `frame_x` / `frame_y` are only transmitted for types where
/// [`frame_important`](crate::tile_sets::frame_important) is true. For every other type the client
/// stores -1, so the constructors here do the same and round-tripping stays exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    pub block: u16,
    pub wall: u16,
    pub frame_x: i16,
    pub frame_y: i16,
    pub liquid: u8,
    pub liquid_kind: Liquid,
    pub color: u8,
    pub wall_color: u8,
    /// 0 = no slope, 1..=4 the four corner slopes. Half bricks use [`TileFlags::HALF_BRICK`].
    pub slope: u8,
    pub flags: TileFlags,
}

impl Default for Tile {
    fn default() -> Self {
        Self::AIR
    }
}

impl Tile {
    /// An empty tile: no block, no wall, no liquid.
    pub const AIR: Tile = Tile {
        block: 0,
        wall: 0,
        frame_x: -1,
        frame_y: -1,
        liquid: 0,
        liquid_kind: Liquid::Water,
        color: 0,
        wall_color: 0,
        slope: 0,
        flags: TileFlags(0),
    };

    /// A plain block with no frame data.
    ///
    /// Debug-asserts the type is not frame-important, since such a tile would arrive at the client
    /// with frames of -1 and render as a corrupt sprite.
    pub fn block(block: u16) -> Self {
        debug_assert!(
            !frame_important(block),
            "tile type {block} is frame-important; use Tile::framed",
        );
        Self {
            block,
            flags: TileFlags(TileFlags::ACTIVE),
            ..Self::AIR
        }
    }

    /// A block of a frame-important type, carrying its position within the multi-tile sprite.
    pub fn framed(block: u16, frame_x: i16, frame_y: i16) -> Self {
        Self {
            block,
            frame_x,
            frame_y,
            flags: TileFlags(TileFlags::ACTIVE),
            ..Self::AIR
        }
    }

    pub fn with_wall(mut self, wall: u16) -> Self {
        self.wall = wall;
        self
    }

    pub fn with_liquid(mut self, kind: Liquid, amount: u8) -> Self {
        self.liquid_kind = kind;
        self.liquid = amount;
        self
    }

    pub const fn is_active(&self) -> bool {
        self.flags.has(TileFlags::ACTIVE)
    }

    pub const fn has_wall(&self) -> bool {
        self.wall != 0
    }

    /// Whether this tile's frames are actually part of its wire representation.
    pub fn frames_are_sent(&self) -> bool {
        self.is_active() && frame_important(self.block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn air_is_the_default() {
        assert_eq!(Tile::default(), Tile::AIR);
        assert!(!Tile::AIR.is_active());
        assert!(!Tile::AIR.has_wall());
    }

    #[test]
    fn plain_blocks_carry_no_frames() {
        let dirt = Tile::block(0);
        assert!(dirt.is_active());
        assert!(!dirt.frames_are_sent());
        assert_eq!((dirt.frame_x, dirt.frame_y), (-1, -1));
    }

    #[test]
    fn framed_blocks_carry_frames() {
        let chest = Tile::framed(21, 36, 0);
        assert!(chest.frames_are_sent());
        assert_eq!((chest.frame_x, chest.frame_y), (36, 0));
    }

    #[test]
    fn flags_round_trip() {
        let mut f = TileFlags::default();
        f.set(TileFlags::WIRE_RED, true);
        f.set(TileFlags::ACTUATOR, true);
        assert!(f.has(TileFlags::WIRE_RED) && f.has(TileFlags::ACTUATOR));
        f.set(TileFlags::WIRE_RED, false);
        assert!(!f.has(TileFlags::WIRE_RED) && f.has(TileFlags::ACTUATOR));
    }

    #[test]
    fn tile_stays_small_enough_for_a_full_world() {
        // 4200x1200 tiles at 16 bytes is about 80 MB, which is the budget we designed for.
        assert!(
            size_of::<Tile>() <= 16,
            "Tile grew to {}",
            size_of::<Tile>()
        );
    }
}
