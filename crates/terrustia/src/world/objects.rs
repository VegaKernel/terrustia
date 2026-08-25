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

/// A townsperson as the world file remembers them.
///
/// Town NPCs were the largest thing the save carried that this server never read *or* wrote: the
/// section was sliced out as an opaque blob and handed back untouched. A real Terraria world's
/// residents were therefore invisible to the server, and anyone who moved in during a session was
/// gone at the next restart along with their name and their house.
#[derive(Debug, Clone, PartialEq)]
pub struct TownNpc {
    /// `netID`, which is the type for everything except the few NPCs with negative variants.
    pub net_id: i32,
    /// The name they were given when they arrived, which is what players know them by.
    pub name: String,
    pub position: (f32, f32),
    pub homeless: bool,
    pub home: (i32, i32),
    /// Which of a townsperson's appearances this one wears.
    pub variation: i32,
    /// Whether they are on their way out because their house was destroyed.
    pub homeless_despawn: bool,
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
    ///
    /// Saving writes the sections it rebuilds in *this* version's shape, not the newest one: the
    /// header is copied verbatim and still says 279, so a chest section written the way 294
    /// writes them would be read back with the wrong field widths and take the rest of the file
    /// with it.
    pub version: i32,
    /// How many slots every chest has, for the versions that stated it once for the whole file.
    ///
    /// `None` from 294 onward, where each chest carries its own count.
    pub chest_slots: Option<i16>,
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
    pub sandstorm_offset: Option<usize>,
    pub army_run_offset: Option<usize>,
    pub combat_book_offset: Option<usize>,
    pub late_downed_run_offset: Option<usize>,
    pub combat_book_two_offset: Option<usize>,
    /// The three hardmode ore tiers, chosen when altars are smashed.
    pub hardmode_ores_offset: Option<usize>,
    /// Where the banner kill counts start, and how many the file has room for.
    pub banner_kills_offset: Option<(usize, usize)>,
    /// Whether the townsfolk section decoded in full.
    ///
    /// Both of these gate *rewriting* the section on save. A section we only partly understood is
    /// carried through as the bytes it arrived as, because rewriting it from a partial read is how
    /// a world loses every resident — or every pylon — to one unrecognised entry.
    pub town_npcs_understood: bool,
    /// Whether every tile entity the section claimed decoded.
    pub tile_entities_understood: bool,
    /// Sections 4 onwards, one blob each, in order.
    ///
    /// Kept separately rather than as one run of bytes so that a section this server *does* model
    /// can be written from its own state while its neighbours are carried through untouched. Only
    /// section 5, the tile entities, is currently rewritten that way; the rest — townsfolk,
    /// pressure plates, the room assignments, the bestiary, the creative powers — pass through.
    ///
    /// The last blob carries the file's footer with it, which is what the game checks a save
    /// against, so it must stay attached to the section it followed.
    pub trailing_sections: Vec<Vec<u8>>,
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
