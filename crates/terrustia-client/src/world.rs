use std::collections::HashMap;

use terrustia_proto::{
    SectionBounds, Tile,
    section::{
        ChestInfo, SECTION_HEIGHT, SECTION_WIDTH, SignInfo, decode_section_stream,
        inflate_section_payload,
    },
};

use crate::error::Result;

/// The client's view of the world: whichever sections have arrived, plus later edits.
///
/// Sections are kept whole rather than merged into one grid, because a client only ever holds the
/// parts of the world it has been sent and needs to know which those are.
#[derive(Debug, Default)]
pub struct ClientWorld {
    pub width: i32,
    pub height: i32,
    pub spawn: (i16, i16),
    pub name: String,
    sections: HashMap<(i32, i32), Vec<Tile>>,
    /// Chests and signs announced in the trailers of the sections we have received.
    ///
    /// A client only learns where these are from section data, so anything wanting to open a chest
    /// has to remember them as they arrive.
    chests: HashMap<i16, ChestInfo>,
    signs: HashMap<i16, SignInfo>,
}

impl ClientWorld {
    /// Which section a tile position belongs to.
    pub fn section_of(x: i32, y: i32) -> (i32, i32) {
        (x.div_euclid(SECTION_WIDTH), y.div_euclid(SECTION_HEIGHT))
    }

    pub fn loaded_sections(&self) -> usize {
        self.sections.len()
    }

    pub fn has_section(&self, sx: i32, sy: i32) -> bool {
        self.sections.contains_key(&(sx, sy))
    }

    /// Absorb a `TileSection` payload.
    pub fn apply_section(&mut self, payload: &[u8]) -> Result<SectionBounds> {
        let stream = inflate_section_payload(payload)?;
        let (bounds, tiles, extras) = decode_section_stream(&stream)?;
        let key = Self::section_of(bounds.x, bounds.y);
        self.sections.insert(key, tiles);
        for chest in extras.chests {
            self.chests.insert(chest.id, chest);
        }
        for sign in extras.signs {
            self.signs.insert(sign.id, sign);
        }
        Ok(bounds)
    }

    /// Every chest this client has been told about.
    pub fn chests(&self) -> impl Iterator<Item = &ChestInfo> {
        self.chests.values()
    }

    /// Every sign this client has been told about.
    pub fn signs(&self) -> impl Iterator<Item = &SignInfo> {
        self.signs.values()
    }

    /// The known chest closest to a tile position.
    pub fn nearest_chest(&self, x: i32, y: i32) -> Option<&ChestInfo> {
        self.chests.values().min_by_key(|c| {
            let (dx, dy) = (i32::from(c.x) - x, i32::from(c.y) - y);
            dx * dx + dy * dy
        })
    }

    /// The tile at a world position, if its section has been received.
    pub fn tile(&self, x: i32, y: i32) -> Option<Tile> {
        let key = Self::section_of(x, y);
        let tiles = self.sections.get(&key)?;
        // Sections at the world edge are narrower, so the stride is derived from the section
        // origin rather than assumed to be the full width.
        let origin_x = key.0 * SECTION_WIDTH;
        let origin_y = key.1 * SECTION_HEIGHT;
        let width = (self.width - origin_x).clamp(0, SECTION_WIDTH);
        if width == 0 {
            return None;
        }
        let (dx, dy) = (x - origin_x, y - origin_y);
        if dx < 0 || dy < 0 || dx >= width {
            return None;
        }
        tiles.get((dy * width + dx) as usize).copied()
    }

    /// Overwrite one tile, for applying edits that arrive after the section did.
    pub fn set_tile(&mut self, x: i32, y: i32, tile: Tile) -> bool {
        let key = Self::section_of(x, y);
        let origin_x = key.0 * SECTION_WIDTH;
        let origin_y = key.1 * SECTION_HEIGHT;
        let width = (self.width - origin_x).clamp(0, SECTION_WIDTH);
        if width == 0 {
            return false;
        }
        let Some(tiles) = self.sections.get_mut(&key) else {
            return false;
        };
        let (dx, dy) = (x - origin_x, y - origin_y);
        if dx < 0 || dy < 0 || dx >= width {
            return false;
        }
        match tiles.get_mut((dy * width + dx) as usize) {
            Some(slot) => {
                *slot = tile;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use terrustia_proto::{
        SectionExtras, Writer,
        section::{decode_section_stream, write_section_stream},
    };

    fn world_with_section(sx: i32, sy: i32, fill: Tile) -> ClientWorld {
        let mut world = ClientWorld {
            width: 800,
            height: 600,
            ..Default::default()
        };
        let bounds = SectionBounds::of_section(sx, sy);
        let mut stream = Writer::new();
        write_section_stream(&mut stream, bounds, &SectionExtras::default(), |_, _| fill);
        let (b, tiles, _) = decode_section_stream(stream.as_slice()).unwrap();
        world
            .sections
            .insert(ClientWorld::section_of(b.x, b.y), tiles);
        world
    }

    #[test]
    fn tiles_resolve_inside_a_loaded_section() {
        let world = world_with_section(1, 1, Tile::block(1));
        assert_eq!(world.tile(250, 200), Some(Tile::block(1)));
        assert_eq!(world.loaded_sections(), 1);
    }

    #[test]
    fn tiles_outside_a_loaded_section_are_unknown() {
        let world = world_with_section(1, 1, Tile::block(1));
        assert_eq!(world.tile(10, 10), None, "section (0,0) was never sent");
        assert!(!world.has_section(0, 0));
    }

    #[test]
    fn an_edit_replaces_a_single_tile() {
        let mut world = world_with_section(0, 0, Tile::block(1));
        assert!(world.set_tile(5, 6, Tile::AIR));
        assert_eq!(world.tile(5, 6), Some(Tile::AIR));
        assert_eq!(world.tile(5, 7), Some(Tile::block(1)));
    }

    #[test]
    fn editing_an_unloaded_tile_reports_failure() {
        let mut world = world_with_section(0, 0, Tile::block(1));
        assert!(!world.set_tile(500, 500, Tile::AIR));
    }

    #[test]
    fn chests_and_signs_are_remembered_from_section_trailers() {
        use terrustia_proto::{SectionExtras, Writer, section::write_section_stream};

        let mut world = ClientWorld {
            width: 800,
            height: 600,
            ..Default::default()
        };
        let bounds = SectionBounds::of_section(0, 0);
        let extras = SectionExtras {
            chests: vec![ChestInfo {
                id: 4,
                x: 50,
                y: 60,
                name: "Loot".into(),
            }],
            signs: vec![SignInfo {
                id: 1,
                x: 10,
                y: 20,
                text: "hi".into(),
            }],
        };
        let mut stream = Writer::new();
        write_section_stream(&mut stream, bounds, &extras, |_, _| Tile::AIR);

        let mut deflated = Vec::new();
        {
            use std::io::Write;
            let mut enc =
                flate2::write::DeflateEncoder::new(&mut deflated, flate2::Compression::default());
            enc.write_all(stream.as_slice()).unwrap();
            enc.finish().unwrap();
        }

        world.apply_section(&deflated).unwrap();
        assert_eq!(world.chests().count(), 1);
        assert_eq!(world.nearest_chest(0, 0).unwrap().name, "Loot");
        assert_eq!(world.signs().next().unwrap().text, "hi");
    }

    #[test]
    fn section_lookup_handles_negative_positions() {
        // div_euclid keeps negatives in the section below zero rather than rounding toward it.
        assert_eq!(ClientWorld::section_of(-1, -1), (-1, -1));
        assert_eq!(ClientWorld::section_of(0, 0), (0, 0));
        assert_eq!(ClientWorld::section_of(200, 150), (1, 1));
    }
}
