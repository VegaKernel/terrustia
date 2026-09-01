//! Item entities lying in the world: packets `21`, `22` and `151`.
//!
//! The lifecycle is split between both sides. The server spawns an item and broadcasts `21`. It
//! then reserves the item for a nearby player with `22`. That player's client performs the pickup
//! locally and reports it with `151`, which the server relays so everyone else removes it too.
//!
//! A client dropping something from its own inventory sends `21` with the index set to
//! [`NEW_ITEM_INDEX`], asking the server to allocate a slot.

use crate::{error::Result, id, item::ItemStack, reader::PacketReader, writer::PacketWriter};

/// Terraria keeps 400 item entity slots.
pub const MAX_ITEMS: usize = 400;

/// The index a client uses to mean "allocate a new slot for this".
pub const NEW_ITEM_INDEX: i16 = 400;

/// The owner value meaning "reserved for nobody".
pub const NO_OWNER: u8 = 255;

/// How old a pickup has to be before the slot picker is willing to throw it away to make room,
/// in ticks. `Item.PickupReplacementTime` (`Item.cs:334`).
pub const PICKUP_REPLACEMENT_TIME: u32 = 1200;

/// How many of the 400 slots a server holds back before it starts recycling rather than handing
/// out the next free one. `Item.SlotsRemainingBeforeEmergencyStackingInMultiplayer`
/// (`Item.cs:336`), applied only when `Main.netMode == 2` (`Item.cs:49798-49802`) - which for this
/// project is always, since it only ever runs as a dedicated server.
pub const SLOTS_RESERVED_BEFORE_RECYCLING: usize = 40;

/// Whether an item is one of the game's "pickups": the hearts and mana stars an enemy scatters by
/// the dozen, their Halloween and Christmas reskins, the Nebula armour boosters and the Mana Cloak
/// star. `ItemID.Sets.IsAPickup` (`ItemID.cs:254`).
///
/// This is the set the slot picker is willing to destroy first when the world is running out of
/// item slots: they drop constantly, they are worth almost nothing individually, and one vanishing
/// is not something a player notices the way a vanishing weapon would be.
pub fn is_a_pickup(id: i32) -> bool {
    matches!(
        id,
        // Heart, Star.
        58 | 184
        // Candy Apple, Soul Cake (Halloween), Candy Cane, Sugar Plum (Christmas).
        | 1734 | 1735 | 1867 | 1868
        // NebulaPickup1..3, ManaCloakStar.
        | 3453 | 3454 | 3455 | 4143
    )
}

/// Packet `21`: an item entity's full state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SyncItem {
    pub index: i16,
    pub position: (f32, f32),
    pub velocity: (f32, f32),
    pub item: ItemStack,
    /// Bits 0 and 1 carry spawn-ownership intent; bits 2 and 3 flag the optional trailers.
    pub flags: u8,
    pub shimmered: bool,
    pub shimmer_time: f32,
    pub enemy_grab_delay: u8,
}

impl SyncItem {
    const HAS_SHIMMER: u8 = 0x04;
    const HAS_GRAB_DELAY: u8 = 0x08;

    /// A plain item lying on the ground, with no shimmer or grab delay.
    pub fn dropped(index: i16, position: (f32, f32), item: ItemStack) -> Self {
        Self {
            index,
            position,
            velocity: (0.0, 0.0),
            item,
            flags: 0,
            shimmered: false,
            shimmer_time: 0.0,
            enemy_grab_delay: 0,
        }
    }

    /// Whether the sender is asking for a new slot rather than updating one.
    pub fn is_new(&self) -> bool {
        self.index == NEW_ITEM_INDEX
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        let index = r.i16()?;
        let position = r.vec2()?;
        let velocity = r.vec2()?;
        let stack = r.i16()?;
        let prefix = r.u8()?;
        let flags = r.u8()?;
        let type_id = i32::from(r.i16()?);

        // Both shimmer fields are gated on one bit, and the game short-circuits, so neither is
        // present when the bit is clear.
        let (shimmered, shimmer_time) = if flags & Self::HAS_SHIMMER != 0 {
            (r.bool()?, r.f32()?)
        } else {
            (false, 0.0)
        };
        let enemy_grab_delay = if flags & Self::HAS_GRAB_DELAY != 0 {
            r.u8()?
        } else {
            0
        };

        Ok(Self {
            index,
            position,
            velocity,
            item: ItemStack::new(type_id, stack, prefix),
            flags,
            shimmered,
            shimmer_time,
            enemy_grab_delay,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        self.encode_as(id::SYNC_ITEM)
    }

    /// The same item, sent as packet `90` (`SpawnInstancedItem`).
    ///
    /// The payload is identical to packet `21`; only the message id differs. The game sends this
    /// instead of the ordinary item sync for a drop that is instanced per client (an expert
    /// treasure bag, dropped by `CommonCode.DropItemLocalPerClientAndSetNPCMoneyTo0`), so that only
    /// the player it is sent to sees or can take it.
    pub fn encode_instanced(&self) -> Result<Vec<u8>> {
        self.encode_as(id::SPAWN_INSTANCED_ITEM)
    }

    fn encode_as(&self, message_id: u8) -> Result<Vec<u8>> {
        let mut flags = self.flags & 0x03;
        if self.shimmered || self.shimmer_time > 0.0 {
            flags |= Self::HAS_SHIMMER;
        }
        if self.enemy_grab_delay > 0 {
            flags |= Self::HAS_GRAB_DELAY;
        }

        let mut w = PacketWriter::new(message_id);
        w.i16(self.index)
            .vec2(self.position.0, self.position.1)
            .vec2(self.velocity.0, self.velocity.1)
            .i16(self.item.stack)
            .u8(self.item.prefix)
            .u8(flags)
            .i16(self.item.id as i16);
        if flags & Self::HAS_SHIMMER != 0 {
            w.bool(self.shimmered).f32(self.shimmer_time);
        }
        if flags & Self::HAS_GRAB_DELAY != 0 {
            w.u8(self.enemy_grab_delay);
        }
        w.finish()
    }
}

/// Packet `22`: reserve an item for a player so their client may pick it up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ItemOwner {
    pub index: i16,
    pub owner: u8,
    pub keep_reservation_ticks: u32,
    pub grab_delay_player: u8,
    pub grab_delay_ticks: u32,
    pub position: (f32, f32),
}

impl ItemOwner {
    pub fn reserve(index: i16, owner: u8, position: (f32, f32)) -> Self {
        Self {
            index,
            owner,
            // Long enough for a client to notice and grab, short enough that a player who walks
            // away does not lock the item forever.
            keep_reservation_ticks: 100,
            grab_delay_player: 0,
            grab_delay_ticks: 0,
            position,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut w = PacketWriter::new(id::ITEM_OWNER);
        w.i16(self.index)
            .u8(self.owner)
            .var_u32(self.keep_reservation_ticks)
            .u8(self.grab_delay_player)
            .var_u32(self.grab_delay_ticks)
            .vec2(self.position.0, self.position.1);
        w.finish()
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = PacketReader::new(payload);
        Ok(Self {
            index: r.i16()?,
            owner: r.u8()?,
            keep_reservation_ticks: r.var_u32()?,
            grab_delay_player: r.u8()?,
            grab_delay_ticks: r.var_u32()?,
            position: r.vec2()?,
        })
    }
}

/// Packet `151`: an item is gone, because someone picked it up or it expired.
pub fn item_despawn(index: i16) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(id::SYNC_ITEM_DESPAWN);
    w.i16(index);
    w.finish()
}

/// Read the index out of a packet `151`.
pub fn decode_item_despawn(payload: &[u8]) -> Result<i16> {
    PacketReader::new(payload).i16()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(frame: &[u8]) -> &[u8] {
        &frame[3..]
    }

    #[test]
    fn a_plain_drop_round_trips_without_optional_trailers() {
        let item = SyncItem::dropped(7, (1600.0, 320.0), ItemStack::new(3, 1, 0));
        let frame = item.encode().unwrap();
        assert_eq!(frame[2], id::SYNC_ITEM);
        // 2 + 8 + 8 + 2 + 1 + 1 + 2, and nothing more.
        assert_eq!(payload(&frame).len(), 24);
        assert_eq!(SyncItem::decode(payload(&frame)).unwrap(), item);
    }

    #[test]
    fn the_instanced_encoding_is_the_same_payload_under_packet_90() {
        let item = SyncItem::dropped(7, (1600.0, 320.0), ItemStack::new(3319, 1, 0));
        let shared = item.encode().unwrap();
        let instanced = item.encode_instanced().unwrap();
        assert_eq!(shared[2], id::SYNC_ITEM);
        assert_eq!(instanced[2], id::SPAWN_INSTANCED_ITEM);
        // Only the message id differs; the bytes a client reads are identical.
        assert_eq!(payload(&shared), payload(&instanced));
        assert_eq!(SyncItem::decode(payload(&instanced)).unwrap(), item);
    }

    #[test]
    fn shimmer_and_grab_delay_add_their_trailers() {
        let mut item = SyncItem::dropped(1, (0.0, 0.0), ItemStack::new(3, 1, 0));
        item.shimmered = true;
        item.shimmer_time = 2.5;
        item.enemy_grab_delay = 30;

        let frame = item.encode().unwrap();
        // The base 24, plus bool + f32, plus the delay byte.
        assert_eq!(payload(&frame).len(), 24 + 5 + 1);

        let decoded = SyncItem::decode(payload(&frame)).unwrap();
        assert!(decoded.shimmered);
        assert_eq!(decoded.shimmer_time, 2.5);
        assert_eq!(decoded.enemy_grab_delay, 30);
    }

    #[test]
    fn the_new_item_sentinel_is_recognised() {
        let item = SyncItem::dropped(NEW_ITEM_INDEX, (0.0, 0.0), ItemStack::new(3, 1, 0));
        assert!(item.is_new());
        assert!(!SyncItem::dropped(0, (0.0, 0.0), ItemStack::new(3, 1, 0)).is_new());
    }

    #[test]
    fn spawn_ownership_bits_survive_but_trailer_bits_are_recomputed() {
        // Bits 0 and 1 are the caller's; 2 and 3 must reflect what is actually written.
        let mut item = SyncItem::dropped(0, (0.0, 0.0), ItemStack::new(3, 1, 0));
        item.flags = 0x03 | 0x04; // claims shimmer without any shimmer state
        let decoded = SyncItem::decode(payload(&item.encode().unwrap())).unwrap();
        assert_eq!(decoded.flags & 0x03, 0x03, "ownership bits kept");
        assert_eq!(decoded.flags & 0x04, 0, "the stale shimmer bit was dropped");
    }

    #[test]
    fn item_owner_round_trips() {
        let owner = ItemOwner::reserve(9, 2, (100.0, 200.0));
        let decoded = ItemOwner::decode(payload(&owner.encode().unwrap())).unwrap();
        assert_eq!(decoded, owner);
    }

    #[test]
    fn despawn_is_just_an_index() {
        let frame = item_despawn(12).unwrap();
        assert_eq!(payload(&frame).len(), 2);
        assert_eq!(decode_item_despawn(payload(&frame)).unwrap(), 12);
    }

    #[test]
    fn truncated_item_packets_error_rather_than_panic() {
        assert!(SyncItem::decode(&[0, 0, 0]).is_err());
        assert!(ItemOwner::decode(&[0]).is_err());
        assert!(decode_item_despawn(&[1]).is_err());
    }
}
