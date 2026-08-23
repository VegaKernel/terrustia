//! Shadow orbs and crimson hearts: what breaking one gives you.
//!
//! The two are the same tile — 31 — told apart by the frame: a heart sits at `frame_x >= 36`.
//! Breaking one is the whole early game of a corruption or crimson world. It is where the first
//! ranged weapon comes from, it is what calls down a meteor, and the third one summons the boss.
//!
//! The reward is not a plain roll. The *first* orb anybody breaks in a world always gives the
//! first entry — the musket or the undertaker — and only after that does it become one in five.
//! That is deliberate on the game's part: it guarantees a gun before it guarantees variety.
//!
//! Transcribed from `WorldGen.CheckOrb` in the 1.4.5.7 build.

/// The tile both are frames of.
pub const ORB_TILE: u16 = 31;

/// A frame at or past this is a crimson heart rather than a shadow orb.
pub const HEART_FRAME: i16 = 36;

/// How many have to break before the evil's boss comes for you.
pub const ORBS_PER_BOSS: u8 = 3;

/// One reward: an item, and how many of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reward {
    pub item: i32,
    pub stack: i16,
}

const fn one(item: i32) -> Reward {
    Reward { item, stack: 1 }
}

/// What a shadow orb gives, in roll order. The first break always takes entry zero.
///
/// Musket and a hundred musket balls, the Vilethorn, the Band of Starpower, the Shadow Orb and
/// the Ball O' Hurt.
pub const SHADOW_ORB: [&[Reward]; 5] = [
    &[
        one(96),
        Reward {
            item: 97,
            stack: 100,
        },
    ],
    &[one(64)],
    &[one(162)],
    &[one(115)],
    &[one(111)],
];

/// ...and what a crimson heart gives: the Undertaker and its ammunition, the Panic Necklace, the
/// Crimson Rod, the Tissue Sample and the Blood Butcherer's material.
pub const CRIMSON_HEART: [&[Reward]; 5] = [
    &[
        one(800),
        Reward {
            item: 97,
            stack: 100,
        },
    ],
    &[one(1256)],
    &[one(802)],
    &[one(3062)],
    &[one(1290)],
];

/// Which of the two a tile is, from its frame.
pub fn is_heart(frame_x: i16) -> bool {
    frame_x >= HEART_FRAME
}

/// The reward for breaking one.
///
/// `already_smashed` is the world's own "has any orb ever been broken" flag, not a count: while
/// it is false the roll is skipped entirely and entry zero is given.
pub fn reward(frame_x: i16, already_smashed: bool, roll: usize) -> &'static [Reward] {
    let table = if is_heart(frame_x) {
        &CRIMSON_HEART
    } else {
        &SHADOW_ORB
    };
    let index = if already_smashed { roll.min(4) } else { 0 };
    table[index]
}

/// The boss the third one wakes.
pub fn boss_for(frame_x: i16) -> u16 {
    if is_heart(frame_x) { 266 } else { 13 }
}

/// What the world says when one breaks and the boss is not due yet.
pub fn omen(orbs_broken: u8) -> &'static str {
    if orbs_broken >= 2 {
        "This is a terrible mistake..."
    } else {
        "You feel an evil presence watching you..."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first orb in a world is never a roll: it is always the gun.
    #[test]
    fn the_first_orb_always_gives_the_weapon() {
        for roll in 0..5 {
            assert_eq!(reward(0, false, roll), SHADOW_ORB[0]);
            assert_eq!(reward(HEART_FRAME, false, roll), CRIMSON_HEART[0]);
        }
    }

    /// After that it is one in five, and every entry is reachable.
    #[test]
    fn later_orbs_roll_across_the_whole_table() {
        for roll in 0..5 {
            assert_eq!(reward(0, true, roll), SHADOW_ORB[roll]);
            assert_eq!(reward(HEART_FRAME, true, roll), CRIMSON_HEART[roll]);
        }
    }

    /// The gun comes with ammunition; nothing else does.
    #[test]
    fn the_weapon_comes_with_a_hundred_rounds() {
        let first = reward(0, false, 0);
        assert_eq!(first.len(), 2);
        assert_eq!(
            first[1],
            Reward {
                item: 97,
                stack: 100
            }
        );
        assert!(reward(0, true, 1).len() == 1);
    }

    /// The frame is the only thing that separates the two, and it separates them everywhere.
    #[test]
    fn the_frame_decides_which_evil_it_is() {
        assert!(!is_heart(0));
        assert!(!is_heart(18));
        assert!(is_heart(36));
        assert!(is_heart(54));
        assert_eq!(boss_for(0), 13, "an orb wakes the Eater of Worlds");
        assert_eq!(boss_for(36), 266, "a heart wakes the Brain of Cthulhu");
        assert_ne!(reward(0, true, 2), reward(36, true, 2));
    }
}
