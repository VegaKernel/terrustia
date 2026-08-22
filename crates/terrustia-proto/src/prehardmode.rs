//! The pre-hardmode NPC roster.
//!
//! Derived from the game's own spawner: `Spawner.SpawnAnNPC` was walked with its brace structure
//! intact, and every type it can choose was collected along with the conditions guarding it.
//!
//! Excluding only `Main.hardMode` is not enough — the Solar Eclipse, Pumpkin Moon and the later
//! invasions are themselves hardmode-only, so their enemies reach a spawn call with no literal
//! hardmode test above it. Mothron arriving in the roster is what made that obvious. Types gated
//! on any of those events are excluded too, and the bosses and helper-reached types added back.
//!
//! The spawner is not the whole story, though. Town NPCs never spawn — they *move in* — so none of
//! them appeared in that walk, and the roster went a long time without a single one. The
//! thirty-one that can arrive before hardmode are listed here too, derived from the
//! `townNPCCanSpawn` conditions in `Main.UpdateTime_SpawnTownNPCs`: everything except the Wizard,
//! Santa, the Truffle, the Steampunker, the Cyborg, the Pirate, the Princess and the rainbow slime,
//! each of which is gated behind hardmode or an event that needs it.
//!
//! This exists so "every pre-hardmode NPC behaves" is something a test can check rather than
//! something a comment claims.

/// Every NPC type that can appear before hardmode (244 of them).
pub const PRE_HARDMODE: [u16; 244] = [
    1, 2, 3, 4, 5, 6, 7, 10, 13, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
    32, 33, 34, 35, 36, 37, 38, 39, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57,
    58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 81, 93, 104, 105, 107, 111,
    113, 114, 115, 117, 123, 124, 143, 144, 145, 147, 148, 149, 150, 159, 161, 162, 164, 166, 167,
    173, 181, 185, 190, 191, 192, 193, 194, 195, 198, 201, 202, 203, 204, 207, 208, 217, 218, 219,
    220, 221, 222, 223, 224, 225, 226, 227, 228, 230, 239, 254, 255, 257, 258, 259, 261, 266, 267,
    287, 289, 290, 291, 292, 293, 294, 295, 296, 297, 298, 299, 300, 301, 302, 303, 316, 337, 353,
    354, 356, 357, 359, 361, 362, 364, 368, 369, 376, 431, 441, 442, 443, 444, 447, 448, 449, 450,
    451, 452, 453, 465, 481, 482, 483, 484, 489, 490, 498, 513, 536, 537, 539, 540, 546, 550, 579,
    580, 581, 588, 589, 590, 591, 592, 593, 594, 601, 602, 604, 605, 606, 607, 608, 610, 611, 615,
    616, 617, 624, 625, 626, 627, 628, 631, 632, 633, 634, 635, 637, 638, 656, 665, 666, 668, 669,
    670, 671, 672, 673, 674, 675, 678, 679, 680, 682, 683, 684, 685, 686, 688, 689, 690, 691, 693,
    694,
];

/// Whether a type can show up before hardmode.
pub fn is_pre_hardmode(npc_type: u16) -> bool {
    PRE_HARDMODE.binary_search(&npc_type).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::npc_data::npc_stats;

    #[test]
    fn the_roster_is_sorted_so_lookups_work() {
        assert!(PRE_HARDMODE.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn every_listed_type_is_one_this_build_defines() {
        for t in PRE_HARDMODE {
            assert!(npc_stats(t).is_some(), "no stats for pre-hardmode type {t}");
        }
    }

    #[test]
    fn the_staples_are_all_present() {
        // A roster missing any of these would mean the derivation went wrong.
        for (t, who) in [
            (1u16, "Blue Slime"),
            (3, "Zombie"),
            (2, "Demon Eye"),
            (21, "Skeleton"),
            (49, "Cave Bat"),
            (6, "Eater of Souls"),
            (4, "Eye of Cthulhu"),
            (50, "King Slime"),
            (13, "Eater of Worlds"),
            (266, "Brain of Cthulhu"),
        ] {
            assert!(is_pre_hardmode(t), "{who} should be pre-hardmode");
        }
    }

    #[test]
    fn hardmode_only_enemies_are_excluded() {
        // Wyvern, Mothron and the Pirate Ship never appear before the Wall of Flesh falls.
        for t in [87u16, 477, 491] {
            assert!(!is_pre_hardmode(t), "type {t} should not be pre-hardmode");
        }
    }
}
