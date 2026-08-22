//! Player inventories — packet 5, `SyncEquipment`.
//!
//! A joining client sends one of these for every slot it holds, and the server's job is twofold:
//! remember them, so somebody who joins later is told what everyone is wearing, and relay them, so
//! everyone already here sees the change.
//!
//! Not every slot is relayed. Piggy bank and safe contents are the player's own business and the
//! game marks them as never leaving the client that owns them; the void vault, oddly, *is* relayed.
//! Getting that wrong either leaks a player's storage to the whole server or loses their loadout,
//! so the map is transcribed from `PlayerItemSlotID` rather than guessed.

use crate::{ItemStack, PacketWriter, Result, id, reader::PacketReader};

/// The slot layout, in the order `PlayerItemSlotID` allocates it.
///
/// Each entry is a run of slots and whether the game relays that run to other clients.
const SLOT_RUNS: [(u16, bool); 17] = [
    (58, true),   // Inventory
    (1, true),    // The item on the cursor
    (20, true),   // Armour and accessories
    (10, true),   // Their dyes
    (5, true),    // Miscellaneous equipment
    (5, true),    // Its dyes
    (200, false), // Piggy bank
    (200, false), // Safe
    (1, false),   // Trash
    (200, false), // Defender's forge
    (200, true),  // Void vault
    (20, true),   // Loadout 1 armour
    (10, true),   // Loadout 1 dyes
    (20, true),   // Loadout 2 armour
    (10, true),   // Loadout 2 dyes
    (20, true),   // Loadout 3 armour
    (10, true),   // Loadout 3 dyes
];

/// How many slots a player has in total.
pub const SLOT_COUNT: u16 = {
    let mut total = 0;
    let mut i = 0;
    while i < SLOT_RUNS.len() {
        total += SLOT_RUNS[i].0;
        i += 1;
    }
    total
};

/// Whether a slot is one the server passes on to other players.
///
/// A slot that is not relayed is still remembered — the server is the authority on what a player
/// is carrying — but nobody else is told about it.
pub fn relayed(slot: u16) -> bool {
    let mut start = 0;
    for (length, relay) in SLOT_RUNS {
        if slot < start + length {
            return relay;
        }
        start += length;
    }
    false
}

/// One slot of one player's inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncEquipment {
    pub player: u8,
    pub slot: u16,
    pub item: ItemStack,
    /// Whether the player has marked it favourite, which stops it being sold or dropped.
    pub favorited: bool,
    /// Whether the slot is blocked — the client's own bookkeeping, passed through unchanged.
    pub blocked: bool,
}

impl SyncEquipment {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        let player = r.u8()?;
        let slot = r.u16()?;
        let stack = r.i16()?;
        let prefix = r.u8()?;
        let item_id = r.i16()?;
        // The flags byte is new enough that an older client may simply stop here.
        let flags = r.u8().unwrap_or(0);
        Ok(Self {
            player,
            slot,
            item: ItemStack {
                id: i32::from(item_id),
                stack,
                prefix,
            },
            favorited: flags & 1 != 0,
            blocked: flags & 2 != 0,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::SYNC_EQUIPMENT);
        w.u8(self.player);
        w.u16(self.slot);
        w.i16(self.item.stack);
        w.u8(self.item.prefix);
        // Item ids travel as sixteen bits here even though the stack keeps them wider.
        w.i16(self.item.id as i16);
        let mut flags = 0u8;
        if self.favorited {
            flags |= 1;
        }
        if self.blocked {
            flags |= 2;
        }
        w.u8(flags);
        w.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slot count is the sum of the runs, and matches what the game allocates.
    #[test]
    fn the_slot_map_adds_up() {
        assert_eq!(
            SLOT_COUNT,
            58 + 1 + 20 + 10 + 5 + 5 + 200 + 200 + 1 + 200 + 200 + 90
        );
    }

    /// Storage a player expects to be private stays private; everything worn does not.
    #[test]
    fn only_the_public_slots_are_relayed() {
        // Inventory, cursor, armour, dyes, miscellaneous: all seen by everyone.
        for slot in [0, 57, 58, 59, 78, 88, 93] {
            assert!(relayed(slot), "slot {slot} should be relayed");
        }
        // Piggy bank, safe, trash and forge: nobody else's business.
        let piggy = 58 + 1 + 20 + 10 + 5 + 5;
        for slot in [piggy, piggy + 199, piggy + 200, piggy + 400, piggy + 401] {
            assert!(!relayed(slot), "slot {slot} should stay private");
        }
        // The void vault is relayed, which is a quirk of the game rather than a mistake here.
        let vault = piggy + 200 + 200 + 1 + 200;
        assert!(relayed(vault), "the void vault is relayed");
        // The loadouts are.
        assert!(relayed(vault + 200), "loadout armour is relayed");
        // Past the end, nothing is.
        assert!(!relayed(SLOT_COUNT), "there is no slot past the last one");
    }

    /// A slot survives a round trip unchanged, flags and all.
    #[test]
    fn a_slot_round_trips() {
        let original = SyncEquipment {
            player: 3,
            slot: 42,
            item: ItemStack {
                id: 3389,
                stack: 7,
                prefix: 81,
            },
            favorited: true,
            blocked: false,
        };
        let bytes = original.encode().unwrap();
        // Skip the two-byte length and the one-byte id.
        let decoded = SyncEquipment::decode(&bytes[3..]).unwrap();
        assert_eq!(decoded, original);
    }

    /// A client that stops before the flags byte is not a protocol error.
    #[test]
    fn an_older_client_without_the_flags_byte_still_decodes() {
        let full = SyncEquipment {
            player: 1,
            slot: 0,
            item: ItemStack {
                id: 1,
                stack: 1,
                prefix: 0,
            },
            favorited: false,
            blocked: false,
        }
        .encode()
        .unwrap();
        // Everything but the trailing flags byte.
        let short = &full[3..full.len() - 1];
        let decoded = SyncEquipment::decode(short).expect("should still decode");
        assert_eq!(decoded.slot, 0);
        assert!(!decoded.favorited);
    }
}
