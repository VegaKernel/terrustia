//! Packets for the interactive world objects: chests, signs and doors.
//!
//! Layouts transcribed from the 1.4.5.7 build; see `docs/protocol-notes.md`.

use crate::{error::Result, id, item::ItemStack, reader::PacketReader, writer::PacketWriter};

/// Packet `31`: a client asking to open the chest anchored at a tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestChestOpen {
    pub x: i16,
    pub y: i16,
}

impl RequestChestOpen {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        Ok(Self {
            x: r.i16()?,
            y: r.i16()?,
        })
    }
}

/// Packet `32`: the contents of one chest slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncChestItem {
    pub chest: i16,
    pub slot: u8,
    pub item: ItemStack,
}

impl SyncChestItem {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        let chest = r.i16()?;
        let slot = r.u8()?;
        let stack = r.i16()?;
        let prefix = r.u8()?;
        // The type travels as an i16 here even though the save format uses an i32.
        let id = i32::from(r.i16()?);
        Ok(Self {
            chest,
            slot,
            item: ItemStack::new(id, stack, prefix),
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::SYNC_CHEST_ITEM);
        w.i16(self.chest)
            .u8(self.slot)
            .i16(self.item.stack)
            .u8(self.item.prefix)
            .i16(self.item.id as i16);
        w.finish()
    }
}

/// Packet `155`: tell a client how many slots a chest has, before its contents arrive.
///
/// Chests gained per-chest capacities in 1.4.5; a client that never receives this assumes the old
/// fixed size and renders the wrong grid.
pub fn sync_chest_size(chest: i16, slots: i16) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::SYNC_CHEST_SIZE);
    w.i16(chest).i16(slots);
    w.finish()
}

/// Packet `33`: which chest a player currently has open.
///
/// Passing `chest = -1` closes whatever the player had open. The name is only sent when it is
/// between 1 and 20 bytes; the leading byte is the length and doubles as a marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncPlayerChest {
    pub chest: i16,
    pub x: i16,
    pub y: i16,
    pub name: Option<String>,
}

impl SyncPlayerChest {
    pub const CLOSED: i16 = -1;

    pub fn closed() -> Self {
        Self {
            chest: Self::CLOSED,
            x: 0,
            y: 0,
            name: None,
        }
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        let chest = r.i16()?;
        let x = r.i16()?;
        let y = r.i16()?;
        let len = r.u8()?;
        // 0 means "no name"; 255 is a marker the client uses for an unnamed chest; anything else
        // over 20 is treated as absent rather than trusted as a length.
        let name = if (1..=20).contains(&len) {
            Some(r.string()?)
        } else {
            None
        };
        Ok(Self { chest, x, y, name })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::SYNC_PLAYER_CHEST);
        w.i16(self.chest).i16(self.x).i16(self.y);
        match self.name.as_deref().filter(|n| (1..=20).contains(&n.len())) {
            Some(name) => {
                w.u8(name.len() as u8).string(name);
            }
            None => {
                w.u8(0);
            }
        }
        w.finish()
    }
}

/// Packet `80`: which chest a player currently has open, relayed to everyone else — real
/// vanilla's own client uses this to show a chest as already in use before it ever tries to open
/// it itself (`MessageBuffer.cs:1886`, `NetMessage.cs:1182-1185`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncPlayerChestIndex {
    pub player: u8,
    /// The chest index, or `-1` when the player has none open.
    pub chest: i16,
}

impl SyncPlayerChestIndex {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        Ok(Self {
            player: r.u8()?,
            chest: r.i16()?,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::SYNC_PLAYER_CHEST_INDEX);
        w.u8(self.player).i16(self.chest);
        w.finish()
    }
}

/// Packet `46`: a client asking to read the sign at a tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestSign {
    pub x: i16,
    pub y: i16,
}

impl RequestSign {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        Ok(Self {
            x: r.i16()?,
            y: r.i16()?,
        })
    }
}

/// Packet `47`: a sign's text, in either direction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignText {
    pub sign: i16,
    pub x: i16,
    pub y: i16,
    pub text: String,
    pub player: u8,
    /// Set when the client is opening the sign for editing rather than just reading it.
    pub editing: u8,
}

impl SignText {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        let sign = r.i16()?;
        let x = r.i16()?;
        let y = r.i16()?;
        let text = r.string()?;
        // The trailing two bytes are absent in some client builds; default them rather than
        // rejecting an otherwise usable packet.
        let player = r.u8().unwrap_or(0);
        let editing = r.u8().unwrap_or(0);
        Ok(Self {
            sign,
            x,
            y,
            text,
            player,
            editing,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::OPEN_SIGN_RESPONSE);
        w.i16(self.sign)
            .i16(self.x)
            .i16(self.y)
            .string(&self.text)
            .u8(self.player)
            .u8(self.editing);
        w.finish()
    }
}

/// Packet `19`: open or close a door.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoorToggle {
    /// 0 open door, 1 close door, 2 close trapdoor, 3 open trapdoor, 4 open tall gate,
    /// 5 close tall gate — `Wiring.cs:1443-1461`. The trapdoor pair reads backwards from the door
    /// and tall-gate ones either side of it: `bool value = type == 387` (the trapdoor is
    /// currently *open*) then `3 - value.ToInt()` gives `2` exactly when this operation is a
    /// close, not an open, verified against source rather than assumed from the door/gate
    /// pattern either side of it.
    pub action: u8,
    pub x: i16,
    pub y: i16,
    /// For a door, which way the player was facing, which decides the door's frame. For a
    /// trapdoor, whether the shift used `playerAbove: true` (`Wiring.cs:1446`,
    /// `WorldGen.ShiftTrapdoor`'s own `playerAbove` argument) — 1 if it did, 0 if the
    /// `playerAbove: false` fallback is what actually succeeded.
    pub direction: u8,
}

impl DoorToggle {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        Ok(Self {
            action: r.u8()?,
            x: r.i16()?,
            y: r.i16()?,
            direction: r.u8()?,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::TOGGLE_DOOR_STATE);
        w.u8(self.action).i16(self.x).i16(self.y).u8(self.direction);
        w.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::Writer;

    fn payload(frame: &[u8]) -> &[u8] {
        &frame[3..]
    }

    #[test]
    fn chest_item_round_trips() {
        let sync = SyncChestItem {
            chest: 12,
            slot: 3,
            item: ItemStack::new(3507, 99, 58),
        };
        let frame = sync.encode().unwrap();
        assert_eq!(frame[2], id::SYNC_CHEST_ITEM);
        assert_eq!(SyncChestItem::decode(payload(&frame)).unwrap(), sync);
    }

    #[test]
    fn an_empty_chest_slot_round_trips_as_empty() {
        let sync = SyncChestItem {
            chest: 0,
            slot: 0,
            item: ItemStack::EMPTY,
        };
        let decoded = SyncChestItem::decode(payload(&sync.encode().unwrap())).unwrap();
        assert!(decoded.item.is_empty());
    }

    #[test]
    fn chest_size_is_four_bytes() {
        let frame = sync_chest_size(5, 40).unwrap();
        assert_eq!(payload(&frame).len(), 4);
        assert_eq!(frame[2], id::SYNC_CHEST_SIZE);
    }

    #[test]
    fn a_named_chest_carries_its_name_twice_over() {
        // The byte before the string is the length, and the string then carries its own prefix.
        // Both are required; sending only one desynchronises the client.
        let sync = SyncPlayerChest {
            chest: 7,
            x: 100,
            y: 200,
            name: Some("Loot".into()),
        };
        let frame = sync.encode().unwrap();
        let p = payload(&frame);
        assert_eq!(p[6], 4, "explicit length byte");
        assert_eq!(p[7], 4, "the string's own length prefix");
        assert_eq!(SyncPlayerChest::decode(p).unwrap(), sync);
    }

    #[test]
    fn an_unnamed_chest_sends_a_zero_length_and_no_string() {
        let sync = SyncPlayerChest {
            chest: 7,
            x: 1,
            y: 2,
            name: None,
        };
        let frame = sync.encode().unwrap();
        assert_eq!(payload(&frame).len(), 7);
        assert_eq!(SyncPlayerChest::decode(payload(&frame)).unwrap(), sync);
    }

    #[test]
    fn an_over_long_chest_name_is_dropped_rather_than_truncated() {
        // Names over 20 bytes are not sent at all; truncating could split a UTF-8 sequence.
        let sync = SyncPlayerChest {
            chest: 1,
            x: 0,
            y: 0,
            name: Some("x".repeat(21)),
        };
        let decoded = SyncPlayerChest::decode(payload(&sync.encode().unwrap())).unwrap();
        assert_eq!(decoded.name, None);
    }

    #[test]
    fn closing_a_chest_uses_index_minus_one() {
        let frame = SyncPlayerChest::closed().encode().unwrap();
        assert_eq!(
            SyncPlayerChest::decode(payload(&frame)).unwrap().chest,
            SyncPlayerChest::CLOSED
        );
    }

    #[test]
    fn sign_text_round_trips() {
        let sign = SignText {
            sign: 3,
            x: 50,
            y: 60,
            text: "beware \u{1F600}".into(),
            player: 2,
            editing: 1,
        };
        let frame = sign.encode().unwrap();
        assert_eq!(SignText::decode(payload(&frame)).unwrap(), sign);
    }

    #[test]
    fn sign_text_tolerates_a_missing_trailer() {
        // Older builds stop after the text; default the trailing bytes instead of failing.
        let mut w = Writer::new();
        w.i16(1).i16(2).i16(3).string("hi");
        let sign = SignText::decode(w.as_slice()).unwrap();
        assert_eq!(sign.text, "hi");
        assert_eq!((sign.player, sign.editing), (0, 0));
    }

    #[test]
    fn door_toggle_round_trips() {
        let door = DoorToggle {
            action: 0,
            x: 400,
            y: 300,
            direction: 1,
        };
        let frame = door.encode().unwrap();
        assert_eq!(frame[2], id::TOGGLE_DOOR_STATE);
        assert_eq!(DoorToggle::decode(payload(&frame)).unwrap(), door);
    }

    #[test]
    fn sync_player_chest_index_round_trips() {
        let sync = SyncPlayerChestIndex {
            player: 5,
            chest: 42,
        };
        let frame = sync.encode().unwrap();
        assert_eq!(frame[2], id::SYNC_PLAYER_CHEST_INDEX);
        assert_eq!(SyncPlayerChestIndex::decode(payload(&frame)).unwrap(), sync);
    }

    #[test]
    fn truncated_object_packets_error_rather_than_panic() {
        assert!(RequestChestOpen::decode(&[1]).is_err());
        assert!(SyncChestItem::decode(&[1, 0, 0]).is_err());
        assert!(DoorToggle::decode(&[0, 1]).is_err());
        assert!(SyncPlayerChest::decode(&[0, 0]).is_err());
        assert!(SyncPlayerChestIndex::decode(&[0]).is_err());
    }
}
