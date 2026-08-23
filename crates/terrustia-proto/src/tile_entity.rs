//! Tile entities: the furniture that remembers something.
//!
//! Most tiles are only a number. A handful carry state a tile cannot hold — what is in an item
//! frame, which hat is on a rack, whether a logic sensor has fired — and those are tile entities,
//! kept beside the world rather than in it.
//!
//! Three of them matter beyond decoration:
//!
//! * A **training dummy** puts an NPC in front of itself when somebody comes near and takes it
//!   away when they leave, which is the only way that NPC ever exists.
//! * A **teleportation pylon** is a tile entity, which is why a pylon network is something the
//!   server has to keep rather than something a client can assert. Pylons are how a 1.4 world is
//!   crossed.
//! * A **logic sensor** is the only *input* wiring has that is not a lever somebody pulled.
//!
//! The rest hold items, and holding them is the whole point: an item frame that forgets what is
//! in it is an empty frame.
//!
//! Note what the wire format does *not* carry. A logic sensor's kind and state are written to the
//! world file but never sent to a client, because a client has no use for either — the sensor's
//! effect reaches it as the tiles the circuit changed. So [`EntityData::LogicSensor`] serialises
//! to nothing at all over the network, and that is correct rather than an omission.

use crate::{
    ItemStack,
    error::Result,
    reader::PacketReader,
    writer::{PacketWriter, Writer},
};

/// The kinds, numbered as `TileEntitiesManager.RegisterAll` registers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    TrainingDummy = 0,
    ItemFrame = 1,
    LogicSensor = 2,
    DisplayDoll = 3,
    WeaponsRack = 4,
    HatRack = 5,
    FoodPlatter = 6,
    TeleportationPylon = 7,
    DeadCellsDisplayJar = 8,
    KiteAnchor = 9,
    CritterAnchor = 10,
}

impl EntityKind {
    pub fn from_id(id: u8) -> Option<Self> {
        Some(match id {
            0 => Self::TrainingDummy,
            1 => Self::ItemFrame,
            2 => Self::LogicSensor,
            3 => Self::DisplayDoll,
            4 => Self::WeaponsRack,
            5 => Self::HatRack,
            6 => Self::FoodPlatter,
            7 => Self::TeleportationPylon,
            8 => Self::DeadCellsDisplayJar,
            9 => Self::KiteAnchor,
            10 => Self::CritterAnchor,
            _ => return None,
        })
    }

    pub fn id(self) -> u8 {
        self as u8
    }

    /// The tile a kind must be standing on to be real.
    ///
    /// A placement naming the wrong tile is a crafted packet, and refusing it is what stops one
    /// hanging an item frame in mid-air or planting a pylon inside a wall. Every kind has one,
    /// the two anchors included — their `IsTileValidForEntity` names 723 and 724.
    pub fn tile(self) -> u16 {
        match self {
            Self::TrainingDummy => 378,
            Self::ItemFrame => 395,
            Self::LogicSensor => 423,
            Self::DisplayDoll => 470,
            Self::WeaponsRack => 471,
            Self::HatRack => 475,
            Self::FoodPlatter => 520,
            Self::TeleportationPylon => 597,
            Self::DeadCellsDisplayJar => 704,
            Self::KiteAnchor => 723,
            Self::CritterAnchor => 724,
        }
    }

    /// The kind that belongs on a tile, if any does.
    ///
    /// This is the direction that matters most: placing the *tile* is how nearly every one of
    /// these comes into existence, so the server has to recognise its own furniture going down.
    pub fn for_tile(tile: u16) -> Option<Self> {
        (0..=10)
            .filter_map(Self::from_id)
            .find(|kind| kind.tile() == tile)
    }

    /// Whether a client may ask for this kind by packet 87.
    ///
    /// Only four may, and the rest is not an oversight in the game: `TileEntity`'s base
    /// `NetPlaceEntityAttempt` does nothing at all, so a placement request naming an item frame,
    /// a mannequin, a hat rack, a food platter, a logic sensor or a display jar is silently
    /// dropped. Those come into being when their *tile* is placed, not by asking.
    ///
    /// Accepting all eleven — which this server did — lets a crafted packet scatter tile entities
    /// through a world at coordinates nothing checks. A fuzzer found three in a saved world that
    /// had none.
    pub fn placeable_by_request(self) -> bool {
        matches!(
            self,
            Self::TrainingDummy
                | Self::WeaponsRack
                | Self::TeleportationPylon
                | Self::KiteAnchor
                | Self::CritterAnchor
        )
    }

    /// The state a freshly placed one of this kind starts with.
    pub fn fresh(self) -> EntityData {
        match self {
            Self::TrainingDummy => EntityData::TrainingDummy { npc: -1 },
            Self::ItemFrame
            | Self::WeaponsRack
            | Self::FoodPlatter
            | Self::DeadCellsDisplayJar => EntityData::Held(ItemStack::EMPTY),
            Self::LogicSensor => EntityData::LogicSensor { check: 0, on: false },
            Self::DisplayDoll => EntityData::DisplayDoll(Box::default()),
            Self::HatRack => EntityData::HatRack(Box::default()),
            Self::TeleportationPylon => EntityData::Pylon,
            Self::KiteAnchor | Self::CritterAnchor => EntityData::Anchor { item: 0 },
        }
    }
}

/// A mannequin's eight armour slots plus its accessory, its dyes, and the pose it stands in.
///
/// One array of nine rather than "eight and one" because the wire format numbers them that way:
/// the first eight share a byte of presence flags and the ninth is folded into a third byte
/// alongside the accessory dye.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DollContents {
    pub equip: [ItemStack; 9],
    pub dyes: [ItemStack; 9],
    /// The one slot that is neither armour nor dye. The game keeps it as an array of one.
    pub misc: [ItemStack; 1],
    pub pose: u8,
}

impl Default for DollContents {
    fn default() -> Self {
        Self {
            equip: [ItemStack::EMPTY; 9],
            dyes: [ItemStack::EMPTY; 9],
            misc: [ItemStack::EMPTY; 1],
            pose: 0,
        }
    }
}

/// A hat rack's two hats and their two dyes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RackContents {
    pub items: [ItemStack; 2],
    pub dyes: [ItemStack; 2],
}

impl Default for RackContents {
    fn default() -> Self {
        Self {
            items: [ItemStack::EMPTY; 2],
            dyes: [ItemStack::EMPTY; 2],
        }
    }
}

/// Whatever a tile entity remembers, which differs entirely by kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityData {
    /// The slot of the NPC this dummy has put out, or -1 for none.
    TrainingDummy { npc: i16 },
    /// One item, on show. Frames, weapon racks, food platters and display jars all work this way.
    Held(ItemStack),
    /// Which condition the sensor watches, and whether it is currently satisfied.
    ///
    /// Never sent to a client: what a client sees of a sensor is whatever its circuit did.
    LogicSensor { check: u8, on: bool },
    /// A mannequin. Boxed because it is by far the largest and every other kind would otherwise
    /// pay for its size.
    DisplayDoll(Box<DollContents>),
    HatRack(Box<RackContents>),
    /// A pylon keeps nothing of its own: which network it belongs to is the tile's own frame.
    Pylon,
    /// A kite or a critter on a leash, remembered as the item it was let out of.
    Anchor { item: i16 },
}

/// One tile entity as the world keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileEntity {
    pub id: i32,
    pub kind: EntityKind,
    pub x: i16,
    pub y: i16,
    pub data: EntityData,
}

impl TileEntity {
    pub fn new(id: i32, kind: EntityKind, x: i16, y: i16) -> Self {
        Self {
            id,
            kind,
            x,
            y,
            data: kind.fresh(),
        }
    }

    /// The NPC a training dummy has out, if it is a dummy and has one.
    pub fn npc(&self) -> Option<u8> {
        match self.data {
            EntityData::TrainingDummy { npc } if npc >= 0 => u8::try_from(npc).ok(),
            _ => None,
        }
    }

    /// Put an NPC out, or take it away.
    pub fn set_npc(&mut self, npc: Option<u8>) {
        if let EntityData::TrainingDummy { npc: slot } = &mut self.data {
            *slot = npc.map_or(-1, i16::from);
        }
    }

    /// The single item a frame, rack, platter or jar is holding.
    pub fn held(&self) -> Option<ItemStack> {
        match self.data {
            EntityData::Held(item) => Some(item),
            _ => None,
        }
    }

    /// Write this entity as the network carries it.
    ///
    /// The world file's form differs: it carries the id and, for a logic sensor, its state. Both
    /// are decided by `network` rather than by two separate writers, as the game does it.
    pub fn write(&self, w: &mut Writer, network: bool) {
        w.u8(self.kind.id());
        if !network {
            w.i32(self.id);
        }
        w.i16(self.x).i16(self.y);
        self.write_data(w, network);
    }

    fn write_data(&self, w: &mut Writer, network: bool) {
        fn item(w: &mut Writer, it: ItemStack) {
            w.i16(it.id as i16).u8(it.prefix).i16(it.stack);
        }

        match &self.data {
            EntityData::TrainingDummy { npc } => {
                w.i16(*npc);
            }
            EntityData::Held(it) => item(w, *it),
            EntityData::LogicSensor { check, on } => {
                // Only the world file wants these; a client has no use for either.
                if !network {
                    w.u8(*check).bool(*on);
                }
            }
            EntityData::DisplayDoll(doll) => {
                // Three bytes of presence flags come first, then only the slots that are filled.
                // A mannequin wearing nothing is therefore four bytes rather than a hundred.
                let mut equip_bits = 0u8;
                let mut dye_bits = 0u8;
                let mut extra_bits = 0u8;
                for i in 0..8 {
                    if !doll.equip[i].is_empty() {
                        equip_bits |= 1 << i;
                    }
                    if !doll.dyes[i].is_empty() {
                        dye_bits |= 1 << i;
                    }
                }
                if !doll.misc[0].is_empty() {
                    extra_bits |= 1;
                }
                if !doll.equip[8].is_empty() {
                    extra_bits |= 1 << 1;
                }
                if !doll.dyes[8].is_empty() {
                    extra_bits |= 1 << 2;
                }
                w.u8(equip_bits).u8(dye_bits).u8(doll.pose).u8(extra_bits);
                for it in doll.equip.iter().chain(&doll.dyes).chain(&doll.misc) {
                    if !it.is_empty() {
                        item(w, *it);
                    }
                }
            }
            EntityData::HatRack(rack) => {
                let mut bits = 0u8;
                for i in 0..2 {
                    if !rack.items[i].is_empty() {
                        bits |= 1 << i;
                    }
                    if !rack.dyes[i].is_empty() {
                        bits |= 1 << (i + 2);
                    }
                }
                w.u8(bits);
                for it in rack.items.iter().chain(&rack.dyes) {
                    if !it.is_empty() {
                        item(w, *it);
                    }
                }
            }
            EntityData::Pylon => {}
            EntityData::Anchor { item } => {
                w.i16(*item);
            }
        }
    }

    /// Read one back, in the same two forms.
    pub fn read(r: &mut PacketReader<'_>, network: bool) -> Result<Self> {
        let Some(kind) = EntityKind::from_id(r.u8()?) else {
            return Err(crate::ProtoError::OutOfRange {
                field: "tile entity kind",
                value: -1,
            });
        };
        let id = if network { 0 } else { r.i32()? };
        let (x, y) = (r.i16()?, r.i16()?);
        let data = Self::read_data(r, kind, network)?;
        Ok(Self {
            id,
            kind,
            x,
            y,
            data,
        })
    }

    fn read_data(r: &mut PacketReader<'_>, kind: EntityKind, network: bool) -> Result<EntityData> {
        fn item(r: &mut PacketReader<'_>) -> Result<ItemStack> {
            let id = i32::from(r.i16()?);
            let prefix = r.u8()?;
            let stack = r.i16()?;
            Ok(ItemStack { id, stack, prefix })
        }

        Ok(match kind {
            EntityKind::TrainingDummy => EntityData::TrainingDummy { npc: r.i16()? },
            EntityKind::ItemFrame
            | EntityKind::WeaponsRack
            | EntityKind::FoodPlatter
            | EntityKind::DeadCellsDisplayJar => EntityData::Held(item(r)?),
            EntityKind::LogicSensor => {
                if network {
                    EntityData::LogicSensor { check: 0, on: false }
                } else {
                    EntityData::LogicSensor {
                        check: r.u8()?,
                        on: r.bool()?,
                    }
                }
            }
            EntityKind::DisplayDoll => {
                let equip_bits = r.u8()?;
                let dye_bits = r.u8()?;
                let pose = r.u8()?;
                let extra_bits = r.u8()?;
                let mut doll = DollContents {
                    pose,
                    ..Default::default()
                };
                for i in 0..8 {
                    if equip_bits >> i & 1 == 1 {
                        doll.equip[i] = item(r)?;
                    }
                }
                if extra_bits >> 1 & 1 == 1 {
                    doll.equip[8] = item(r)?;
                }
                for i in 0..8 {
                    if dye_bits >> i & 1 == 1 {
                        doll.dyes[i] = item(r)?;
                    }
                }
                if extra_bits >> 2 & 1 == 1 {
                    doll.dyes[8] = item(r)?;
                }
                if extra_bits & 1 == 1 {
                    doll.misc[0] = item(r)?;
                }
                EntityData::DisplayDoll(Box::new(doll))
            }
            EntityKind::HatRack => {
                let bits = r.u8()?;
                let mut rack = RackContents::default();
                for i in 0..2 {
                    if bits >> i & 1 == 1 {
                        rack.items[i] = item(r)?;
                    }
                }
                for i in 0..2 {
                    if bits >> (i + 2) & 1 == 1 {
                        rack.dyes[i] = item(r)?;
                    }
                }
                EntityData::HatRack(Box::new(rack))
            }
            EntityKind::TeleportationPylon => EntityData::Pylon,
            EntityKind::KiteAnchor | EntityKind::CritterAnchor => {
                EntityData::Anchor { item: r.i16()? }
            }
        })
    }
}

/// Packet `86`: one tile entity's whole state, or word that it is gone.
///
/// Until this is sent an entity does not exist as far as any client is concerned: an item frame
/// hangs empty, a mannequin stands bare, and a pylon is a decoration you cannot travel to.
pub fn share(entity: &TileEntity) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(crate::id::TILE_ENTITY_SHARING);
    w.i32(entity.id).bool(true);
    entity.write(&mut w, true);
    w.finish()
}

/// Packet `86` with nothing in it: this entity is no longer there.
pub fn unshare(id: i32) -> Result<Vec<u8>> {
    let mut w = PacketWriter::new(crate::id::TILE_ENTITY_SHARING);
    w.i32(id).bool(false);
    w.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The numbering is the game's own registration order.
    #[test]
    fn the_kinds_are_numbered_as_the_game_registers_them() {
        assert_eq!(EntityKind::from_id(0), Some(EntityKind::TrainingDummy));
        assert_eq!(EntityKind::from_id(7), Some(EntityKind::TeleportationPylon));
        assert_eq!(EntityKind::from_id(10), Some(EntityKind::CritterAnchor));
        assert_eq!(EntityKind::from_id(11), None);
        for id in 0..11u8 {
            assert_eq!(EntityKind::from_id(id).unwrap().id(), id);
        }
    }

    /// Every kind with a home names a tile this build has, and no two share one.
    #[test]
    fn each_kind_has_its_own_tile() {
        let mut seen = std::collections::HashSet::new();
        for id in 0..11u8 {
            let kind = EntityKind::from_id(id).expect("a kind");
            let tile = kind.tile();
            assert!(
                tile < crate::tile_sets::TILE_COUNT,
                "{kind:?} names tile {tile}, past the end of the table"
            );
            assert!(seen.insert(tile), "{kind:?} shares tile {tile}");
        }
    }

    /// The anchors have tiles of their own, which is easy to get wrong: they are the only two
    /// whose tile is named in `IsTileValidForEntity` rather than anywhere obvious, and treating
    /// them as homeless skips the check that stops a crafted packet planting them anywhere.
    #[test]
    fn the_anchors_have_homes_too() {
        assert_eq!(EntityKind::KiteAnchor.tile(), 723);
        assert_eq!(EntityKind::CritterAnchor.tile(), 724);
        assert_eq!(EntityKind::for_tile(723), Some(EntityKind::KiteAnchor));
        assert_eq!(EntityKind::for_tile(724), Some(EntityKind::CritterAnchor));
    }

    /// Placing the tile is how nearly all of these arrive, so the lookup has to go both ways.
    #[test]
    fn a_tile_names_its_entity() {
        for id in 0..11u8 {
            let kind = EntityKind::from_id(id).unwrap();
            assert_eq!(
                EntityKind::for_tile(kind.tile()),
                Some(kind),
                "{kind:?} should be findable from its own tile"
            );
        }
        assert_eq!(EntityKind::for_tile(1), None, "stone holds nothing");
    }

    /// Only four kinds may be asked for over the network. The rest come from their tile.
    #[test]
    fn most_kinds_cannot_be_asked_for() {
        assert!(EntityKind::TeleportationPylon.placeable_by_request());
        assert!(EntityKind::TrainingDummy.placeable_by_request());
        assert!(EntityKind::WeaponsRack.placeable_by_request());
        assert!(EntityKind::KiteAnchor.placeable_by_request());
        assert!(!EntityKind::ItemFrame.placeable_by_request());
        assert!(!EntityKind::DisplayDoll.placeable_by_request());
        assert!(!EntityKind::HatRack.placeable_by_request());
        assert!(!EntityKind::LogicSensor.placeable_by_request());
        assert!(!EntityKind::FoodPlatter.placeable_by_request());
        assert!(!EntityKind::DeadCellsDisplayJar.placeable_by_request());
    }

    fn round_trip(entity: &TileEntity, network: bool) -> TileEntity {
        let mut w = Writer::new();
        entity.write(&mut w, network);
        let bytes = w.into_bytes();
        let mut r = PacketReader::new(&bytes);
        let back = TileEntity::read(&mut r, network).expect("it should read back");
        assert_eq!(r.remaining(), 0, "every byte written should be read");
        back
    }

    /// Every kind survives the trip, in both forms.
    #[test]
    fn each_kind_round_trips() {
        for id in 0..11u8 {
            let kind = EntityKind::from_id(id).unwrap();
            let entity = TileEntity::new(7, kind, 100, 200);
            for network in [false, true] {
                let back = round_trip(&entity, network);
                assert_eq!(back.kind, entity.kind);
                assert_eq!((back.x, back.y), (100, 200));
                if !network {
                    assert_eq!(back.id, 7, "the file form carries the id");
                }
            }
        }
    }

    /// An item frame remembers what is in it, prefix and all.
    #[test]
    fn a_frame_holds_its_item() {
        let mut frame = TileEntity::new(1, EntityKind::ItemFrame, 10, 20);
        frame.data = EntityData::Held(ItemStack {
            id: 3507,
            stack: 1,
            prefix: 81,
        });
        let back = round_trip(&frame, true);
        assert_eq!(back.held(), frame.held());
        assert_eq!(back.held().unwrap().prefix, 81);
    }

    /// A mannequin sends only the slots that are filled, and reads them back into the right ones.
    #[test]
    fn a_mannequin_sends_only_what_it_wears() {
        let mut doll = TileEntity::new(2, EntityKind::DisplayDoll, 10, 20);
        let mut contents = DollContents {
            pose: 3,
            ..Default::default()
        };
        contents.equip[0] = ItemStack::new(10, 1, 0);
        contents.equip[8] = ItemStack::new(20, 1, 0);
        contents.dyes[4] = ItemStack::new(30, 1, 0);
        contents.misc[0] = ItemStack::new(40, 1, 0);
        doll.data = EntityData::DisplayDoll(Box::new(contents.clone()));

        let mut bare = TileEntity::new(2, EntityKind::DisplayDoll, 10, 20);
        bare.data = EntityData::DisplayDoll(Box::default());
        let (mut dressed_bytes, mut bare_bytes) = (Writer::new(), Writer::new());
        doll.write(&mut dressed_bytes, true);
        bare.write(&mut bare_bytes, true);
        assert!(
            dressed_bytes.len() > bare_bytes.len(),
            "an empty mannequin should be the shorter packet"
        );

        let back = round_trip(&doll, true);
        let EntityData::DisplayDoll(got) = back.data else {
            panic!("it should still be a mannequin")
        };
        assert_eq!(*got, contents);
    }

    /// A hat rack likewise.
    #[test]
    fn a_hat_rack_keeps_its_hats_apart_from_its_dyes() {
        let mut rack = TileEntity::new(3, EntityKind::HatRack, 10, 20);
        let contents = RackContents {
            items: [ItemStack::new(11, 1, 0), ItemStack::EMPTY],
            dyes: [ItemStack::EMPTY, ItemStack::new(22, 1, 0)],
        };
        rack.data = EntityData::HatRack(Box::new(contents));
        let back = round_trip(&rack, true);
        let EntityData::HatRack(got) = back.data else {
            panic!("it should still be a rack")
        };
        assert_eq!(*got, contents);
    }

    /// A logic sensor's state goes to the world file and to nobody else.
    #[test]
    fn a_sensor_tells_the_file_but_not_the_network() {
        let mut sensor = TileEntity::new(4, EntityKind::LogicSensor, 10, 20);
        sensor.data = EntityData::LogicSensor { check: 5, on: true };

        let back = round_trip(&sensor, false);
        assert_eq!(back.data, EntityData::LogicSensor { check: 5, on: true });

        let over_the_wire = round_trip(&sensor, true);
        assert_eq!(
            over_the_wire.data,
            EntityData::LogicSensor { check: 0, on: false },
            "the network form carries neither, so it reads back as the default"
        );
    }

    /// A dummy remembers which NPC it has out, and that having none is not slot zero.
    #[test]
    fn a_dummy_distinguishes_no_npc_from_npc_zero() {
        let mut dummy = TileEntity::new(5, EntityKind::TrainingDummy, 10, 20);
        assert_eq!(dummy.npc(), None, "a fresh dummy has nobody out");
        dummy.set_npc(Some(0));
        assert_eq!(dummy.npc(), Some(0));
        assert_eq!(round_trip(&dummy, true).npc(), Some(0));
        dummy.set_npc(None);
        assert_eq!(dummy.npc(), None);
        assert_eq!(round_trip(&dummy, true).npc(), None);
    }

    /// The sharing packet is what makes an entity exist for a client at all.
    #[test]
    fn sharing_carries_the_whole_entity() {
        let mut frame = TileEntity::new(9, EntityKind::ItemFrame, 33, 44);
        frame.data = EntityData::Held(ItemStack::new(3507, 1, 0));
        let bytes = share(&frame).unwrap();
        // length prefix, message id, then the body.
        assert_eq!(bytes[2], crate::id::TILE_ENTITY_SHARING);
        let mut r = PacketReader::new(&bytes[3..]);
        assert_eq!(r.i32().unwrap(), 9);
        assert!(r.bool().unwrap(), "present");
        let back = TileEntity::read(&mut r, true).unwrap();
        assert_eq!(back.held(), frame.held());
        assert_eq!((back.x, back.y), (33, 44));
    }

    /// And the absence packet says only that it has gone.
    #[test]
    fn unsharing_says_only_that_it_is_gone() {
        let bytes = unshare(9).unwrap();
        let mut r = PacketReader::new(&bytes[3..]);
        assert_eq!(r.i32().unwrap(), 9);
        assert!(!r.bool().unwrap());
        assert_eq!(r.remaining(), 0);
    }
}
