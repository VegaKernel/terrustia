//! Locked chests and doors, and what it takes to open one.
//!
//! A locked chest is not a different tile — it is the same chest shifted along its frame strip, so
//! unlocking one is a matter of moving it back. Which offset depends on the chest: an ordinary
//! dungeon or golden chest shifts by one frame, and a temple chest by five, because the temple's
//! locked and unlocked frames sit further apart.
//!
//! Two of them are gated on Plantera. That is not decoration: the biome chests and the temple are
//! the reward for beating her, and a server that let them open early would hand out the whole of
//! the game's late progression at once.

/// Chest tiles: the ordinary one and the temple's.
pub const CHEST: u16 = 21;
pub const CHEST_2: u16 = 467;
/// A door, which locks the same way.
pub const DOOR_CLOSED: u16 = 10;

/// How far along its frame strip a chest moves when unlocked, and whether Plantera gates it.
///
/// Returns `None` for a chest that has no lock at all, which is most of them.
pub fn unlock_shift(block: u16, style: i32) -> Option<(i16, bool)> {
    match block {
        CHEST => match style {
            // A dungeon chest and a golden one: one frame along.
            2 | 4 | 36 | 38 | 40 => Some((36, false)),
            // The five biome chests, which need Plantera down.
            23..=27 => Some((180, true)),
            _ => None,
        },
        CHEST_2 => (style == 13).then_some((36, true)),
        _ => None,
    }
}

/// The same in reverse: locking a chest again.
pub fn lock_shift(block: u16, style: i32) -> Option<(i16, bool)> {
    match block {
        CHEST => match style {
            1 | 3 | 35 | 37 | 39 => Some((36, false)),
            18..=22 => Some((180, true)),
            _ => None,
        },
        CHEST_2 => (style == 12).then_some((36, true)),
        _ => None,
    }
}

/// What a lock request is asking for. The numbering is the packet's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockAction {
    UnlockChest,
    UnlockDoor,
    LockChest,
}

impl LockAction {
    pub fn from_id(id: u8) -> Option<Self> {
        match id {
            1 => Some(Self::UnlockChest),
            2 => Some(Self::UnlockDoor),
            3 => Some(Self::LockChest),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An ordinary chest has no lock and cannot be unlocked.
    #[test]
    fn a_plain_chest_has_no_lock() {
        assert_eq!(unlock_shift(CHEST, 0), None);
        assert_eq!(
            unlock_shift(CHEST, 1),
            None,
            "already locked, not unlockable"
        );
        assert_eq!(unlock_shift(999, 2), None, "not a chest at all");
    }

    /// A dungeon chest opens with a key and shifts one frame.
    #[test]
    fn a_dungeon_chest_shifts_one_frame() {
        assert_eq!(unlock_shift(CHEST, 2), Some((36, false)));
        // And locking it again is the reverse.
        assert_eq!(lock_shift(CHEST, 1), Some((36, false)));
    }

    /// The biome chests and the temple both wait for Plantera.
    #[test]
    fn the_late_chests_wait_for_plantera() {
        for style in 23..=27 {
            assert_eq!(
                unlock_shift(CHEST, style),
                Some((180, true)),
                "style {style}"
            );
        }
        assert_eq!(unlock_shift(CHEST_2, 13), Some((36, true)), "the temple");
        // A dungeon chest does not.
        assert_eq!(unlock_shift(CHEST, 2).map(|(_, gated)| gated), Some(false));
    }

    /// Locking and unlocking are exact inverses: a chest that unlocks by N locks by N.
    #[test]
    fn locking_undoes_unlocking() {
        for style in 0..60 {
            if let Some((shift, gated)) = unlock_shift(CHEST, style) {
                // The locked style is the unlocked one minus the shift in frame terms.
                let locked_style = style - i32::from(shift) / 36;
                assert_eq!(
                    lock_shift(CHEST, locked_style),
                    Some((shift, gated)),
                    "style {style} unlocks by {shift} but does not lock back"
                );
            }
        }
    }

    /// The packet's action numbers are the game's.
    #[test]
    fn the_actions_are_numbered_as_the_packet_has_them() {
        assert_eq!(LockAction::from_id(1), Some(LockAction::UnlockChest));
        assert_eq!(LockAction::from_id(2), Some(LockAction::UnlockDoor));
        assert_eq!(LockAction::from_id(3), Some(LockAction::LockChest));
        assert_eq!(LockAction::from_id(0), None);
        assert_eq!(LockAction::from_id(4), None);
    }
}
