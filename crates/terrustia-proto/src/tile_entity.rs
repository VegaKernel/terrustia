//! Tile entities: the furniture that remembers something.
//!
//! Most tiles are only a number. A handful carry state a tile cannot hold — what is in an item
//! frame, which hat is on a rack, whether a logic sensor has fired — and those are tile entities,
//! kept beside the world rather than in it.
//!
//! Two of them matter beyond decoration. A training dummy is a tile entity that puts an NPC in
//! front of itself and takes it away again when you walk off, which is the only way that NPC ever
//! exists. And a teleportation pylon is a tile entity too, which is why a pylon network is
//! something the server has to keep rather than something a client can assert.

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

    /// The tile a kind must be standing on to be real.
    ///
    /// A placement naming the wrong tile is a crafted packet, and refusing it is what stops one
    /// hanging an item frame in mid-air or planting a pylon inside a wall.
    pub fn tile(self) -> Option<u16> {
        Some(match self {
            Self::TrainingDummy => 378,
            Self::ItemFrame => 395,
            Self::LogicSensor => 423,
            Self::DisplayDoll => 470,
            Self::WeaponsRack => 471,
            Self::HatRack => 475,
            Self::FoodPlatter => 520,
            Self::TeleportationPylon => 597,
            Self::DeadCellsDisplayJar => 704,
            // The two anchors ride other tiles and have no one home of their own.
            Self::KiteAnchor | Self::CritterAnchor => return None,
        })
    }
}

/// One tile entity as the world keeps it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileEntity {
    pub id: i32,
    pub kind: EntityKind,
    pub x: i16,
    pub y: i16,
    /// The NPC a training dummy has put out, if it has one.
    pub npc: Option<u8>,
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
    }

    /// Every kind with a home names a tile this build has, and no two share one.
    #[test]
    fn each_kind_has_its_own_tile() {
        let mut seen = std::collections::HashSet::new();
        for id in 0..11u8 {
            let kind = EntityKind::from_id(id).expect("a kind");
            let Some(tile) = kind.tile() else {
                continue;
            };
            assert!(
                tile < crate::tile_sets::TILE_COUNT,
                "{kind:?} names tile {tile}, past the end of the table"
            );
            assert!(seen.insert(tile), "{kind:?} shares tile {tile}");
        }
    }

    /// The anchors have no tile of their own, and say so rather than guessing one.
    #[test]
    fn the_anchors_have_no_home() {
        assert_eq!(EntityKind::KiteAnchor.tile(), None);
        assert_eq!(EntityKind::CritterAnchor.tile(), None);
    }
}
