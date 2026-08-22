use terrustia_proto::{
    ItemStack,
    section::{ChestInfo, SignInfo},
};

/// Terraria addresses chests and signs by their index in a fixed array, and those indices travel
/// on the wire, so a removed entry has to leave a hole rather than shift its neighbours.
pub const MAX_CHESTS: usize = 8000;
pub const MAX_SIGNS: usize = 1000;

/// Slots in a chest placed at runtime. Loaded chests keep whatever size the save recorded.
pub const DEFAULT_CHEST_SLOTS: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chest {
    pub x: i16,
    pub y: i16,
    pub name: String,
    pub items: Vec<ItemStack>,
}

impl Chest {
    pub fn empty_at(x: i16, y: i16) -> Self {
        Self {
            x,
            y,
            name: String::new(),
            items: vec![ItemStack::EMPTY; DEFAULT_CHEST_SLOTS],
        }
    }

    pub fn info(&self, id: i16) -> ChestInfo {
        ChestInfo {
            id,
            x: self.x,
            y: self.y,
            name: self.name.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sign {
    pub x: i16,
    pub y: i16,
    pub text: String,
}

impl Sign {
    pub fn info(&self, id: i16) -> SignInfo {
        SignInfo {
            id,
            x: self.x,
            y: self.y,
            text: self.text.clone(),
        }
    }
}

/// Byte ranges of a loaded save that this server does not model.
///
/// Saving re-serialises only the header prefix, tiles, chests and signs. Everything else — NPCs,
/// tile entities, pressure plates, the town manager, the bestiary, creative powers — is written
/// back exactly as it was read, so opening a world here and saving it cannot quietly discard the
/// parts we do not understand.
#[derive(Debug, Clone, Default)]
pub struct PreservedWorld {
    /// Format version of the file this came from.
    pub version: i32,
    pub revision: u32,
    pub favorite: u64,
    /// The entire world-header section, verbatim.
    ///
    /// Saving copies this and patches only the handful of fields the server actually changes.
    /// Re-serialising the whole header would mean transcribing 138 further fields and five nested
    /// sub-loaders, every one of which would fail silently if it drifted.
    pub header_bytes: Vec<u8>,
    /// Offsets within `header_bytes` of the mutable clock fields.
    pub time_offset: usize,
    pub day_time_offset: usize,
    pub moon_phase_offset: usize,
    /// Offsets of the progression flags, which the server also changes.
    ///
    /// `None` where a world's header did not reach that far, in which case the flag is kept in
    /// memory for the session and simply not written back.
    pub progress_offset: Option<usize>,
    pub hard_mode_offset: Option<usize>,
    pub altar_offset: Option<usize>,
    pub orb_count_offset: Option<usize>,
    pub downed_run_offset: Option<usize>,
    pub tower_run_offset: Option<usize>,
    pub rain_offset: Option<usize>,
    pub wind_offset: Option<usize>,
    /// Original absolute offsets of section 4 onwards.
    ///
    /// The bytes are written back unchanged, so the new offsets are these shifted by however much
    /// the rewritten sections before them grew or shrank.
    pub trailing_offsets: Vec<i32>,
    /// Everything from the start of section 4 to the end of the file, including the footer.
    pub trailing_bytes: Vec<u8>,
    /// The save's own frame-importance table, which decides how its tiles were encoded.
    pub importance: Vec<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_chest_has_default_slots_and_no_name() {
        let chest = Chest::empty_at(10, 20);
        assert_eq!(chest.items.len(), DEFAULT_CHEST_SLOTS);
        assert!(chest.items.iter().all(ItemStack::is_empty));
        assert!(chest.name.is_empty());
    }

    #[test]
    fn info_carries_the_index_as_the_wire_id() {
        let chest = Chest::empty_at(1, 2);
        assert_eq!(chest.info(7).id, 7);
        let sign = Sign {
            x: 3,
            y: 4,
            text: "hi".into(),
        };
        assert_eq!(sign.info(2).text, "hi");
    }
}
