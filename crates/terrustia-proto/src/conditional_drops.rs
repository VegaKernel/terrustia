//! Loot that depends on more than the thing that died.
//!
//! The flat table in [`crate::npc_drops`] covers what an enemy drops unconditionally. This is the
//! other half: drops that need to know what mode the world is in, how far through the game it is,
//! or where the kill happened.
//!
//! Three kinds matter enough to be worth having. A boss in expert mode drops a treasure bag
//! *instead of* its ordinary loot, which is the whole shape of expert progression. Every boss drops
//! a trophy one time in ten and a mask one in seven, which is what a trophy room is made of. And a
//! good deal of hardmode's crafting material — souls, dust, horns — only drops once the wall has
//! fallen, so dropping it before then would break the progression it exists to gate.

/// What the world was like when something died.
#[derive(Debug, Clone, Copy, Default)]
pub struct Conditions {
    /// Expert or above, which is what turns a boss's loot into a bag.
    pub expert: bool,
    /// Master, which is above expert. A handful of drops are rolled three ways rather than two.
    pub master: bool,
    pub hard_mode: bool,
    pub downed_plantera: bool,
    /// Where it happened, for the drops that only come from one biome.
    pub in_hallow: bool,
    pub in_corruption: bool,
    pub in_crimson: bool,
    /// Below the rock layer, which is where the souls live.
    pub underground: bool,
}

/// One conditional drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conditional {
    pub item: u16,
    pub one_in: u32,
    pub min: i16,
    pub max: i16,
}

const fn always(item: u16) -> Conditional {
    Conditional {
        item,
        one_in: 1,
        min: 1,
        max: 1,
    }
}

const fn sometimes(item: u16, one_in: u32) -> Conditional {
    Conditional {
        item,
        one_in,
        min: 1,
        max: 1,
    }
}

const fn a_few(item: u16, one_in: u32, min: i16, max: i16) -> Conditional {
    Conditional {
        item,
        one_in,
        min,
        max,
    }
}

/// The treasure bag a boss drops in expert mode, if it has one.
pub fn treasure_bag(npc_type: u16) -> Option<u16> {
    Some(match npc_type {
        50 => 3318,        // King Slime
        4 => 3319,         // Eye of Cthulhu
        13..=15 => 3320,   // Eater of Worlds, any segment
        266 => 3321,       // Brain of Cthulhu
        222 => 3322,       // Queen Bee
        35 | 36 => 3323,   // Skeletron
        113 => 3324,       // Wall of Flesh
        134 => 3325,       // The Destroyer
        125 | 126 => 3326, // The Twins
        127 => 3327,       // Skeletron Prime
        262 => 3328,       // Plantera
        245 => 3329,       // Golem
        370 => 3330,       // Duke Fishron
        439 => 3331,       // Lunatic Cultist
        398 => 3332,       // Moon Lord
        _ => return None,
    })
}

/// A boss's trophy, which is a one-in-ten drop whatever the mode.
pub fn trophy(npc_type: u16) -> Option<u16> {
    Some(match npc_type {
        4 => 1360,
        13..=15 => 1361,
        266 => 1362,
        35 | 36 => 1363,
        222 => 1364,
        113 => 1365,
        134 => 1366,
        127 => 1367,
        125 => 1368,
        126 => 1369,
        262 => 1370,
        245 => 1371,
        50 => 2489,
        370 => 2589,
        439 => 3357,
        395 => 3358,
        _ => return None,
    })
}

/// Everything that only drops under some condition.
///
/// Returns an empty list for most types, which is the point: a condition that never applies costs
/// a match arm and nothing else.
pub fn conditional(npc_type: u16, at: Conditions) -> Vec<Conditional> {
    let mut out = Vec::new();

    // Expert turns a boss's loot into a bag. The ordinary drops still happen; the bag is extra,
    // and it is what carries the expert-only accessory.
    if at.expert
        && let Some(bag) = treasure_bag(npc_type)
    {
        out.push(always(bag));
    }
    if let Some(trophy) = trophy(npc_type) {
        out.push(sometimes(trophy, 10));
    }

    // Hardmode's crafting materials. Every one of these is what gates the tier above it, so
    // dropping any of them early would let a world skip a step.
    if at.hard_mode {
        match npc_type {
            // Souls of Night: anything in the evil underground.
            _ if at.underground && (at.in_corruption || at.in_crimson) => {
                out.push(a_few(547, 5, 1, 2));
            }
            _ => {}
        }
        if at.underground && at.in_hallow {
            out.push(a_few(548, 5, 1, 2));
        }
        match npc_type {
            // A wyvern is the only source of Soul of Flight.
            87 => out.push(a_few(754, 1, 20, 40)),
            // The hallow's own three.
            75 => {
                out.push(a_few(521, 2, 1, 5));
                out.push(sometimes(494, 25));
            }
            // A unicorn's horn.
            86 => out.push(sometimes(1327, 5)),
            // Mimics, which are the point of a hardmode chest.
            55 => out.push(sometimes(671, 1)),
            _ => {}
        }
    }

    // Plantera's death opens the temple, and the key is what opens it.
    if at.downed_plantera && npc_type == 262 {
        out.push(always(1293));
    }
    if !at.expert {
        out.extend(classic_only(npc_type));
    }
    out.extend(by_mode(npc_type, at));
    out
}

/// What a boss drops when the world is *not* in expert.
///
/// In expert the treasure bag replaces all of this, which is why every one of these rules is
/// gated the same way: a classic world gets the loot directly, and an expert world gets the bag
/// that contains it. Most servers run classic, so leaving these out left most bosses dropping
/// nothing but coins and a trophy.
fn classic_only(npc_type: u16) -> Vec<Conditional> {
    match npc_type {
        4 => vec![
            a_few(2112, 7, 1, 1),
            a_few(1299, 40, 1, 1),
            a_few(47, 1, 20, 50),
        ],
        13 => vec![
            a_few(56, 1, 20, 60),
            a_few(994, 20, 1, 1),
            a_few(2111, 7, 1, 1),
        ],
        14 => vec![
            a_few(56, 1, 20, 60),
            a_few(994, 20, 1, 1),
            a_few(2111, 7, 1, 1),
        ],
        15 => vec![
            a_few(56, 1, 20, 60),
            a_few(994, 20, 1, 1),
            a_few(2111, 7, 1, 1),
        ],
        113 => vec![a_few(2105, 7, 1, 1), a_few(367, 1, 1, 1)],
        127 => vec![
            a_few(2107, 7, 1, 1),
            a_few(1225, 1, 15, 30),
            a_few(547, 1, 25, 40),
        ],
        134 => vec![
            a_few(2113, 7, 1, 1),
            a_few(1225, 1, 15, 30),
            a_few(548, 1, 25, 40),
        ],
        222 => vec![
            a_few(2108, 7, 1, 1),
            a_few(1132, 3, 1, 1),
            a_few(1170, 15, 1, 1),
            a_few(2502, 20, 1, 1),
            a_few(5483, 15, 1, 1),
            a_few(1130, 4, 10, 30),
            a_few(2431, 1, 17, 30),
        ],
        245 => vec![
            a_few(2110, 7, 1, 1),
            a_few(1294, 4, 1, 1),
            a_few(6158, 6, 1, 1),
            a_few(2218, 1, 4, 8),
        ],
        266 => vec![
            a_few(880, 1, 40, 90),
            a_few(2104, 7, 1, 1),
            a_few(3060, 20, 1, 1),
        ],
        370 => vec![a_few(2588, 7, 1, 1), a_few(2609, 15, 1, 1)],
        398 => vec![
            a_few(3373, 7, 1, 1),
            a_few(4469, 10, 1, 1),
            a_few(3384, 1, 1, 1),
            a_few(3460, 1, 70, 90),
        ],
        551 => vec![a_few(3863, 7, 1, 1), a_few(3883, 4, 1, 1)],
        668 => vec![
            a_few(5109, 7, 1, 1),
            a_few(5098, 3, 1, 1),
            a_few(5101, 3, 1, 1),
            a_few(5113, 3, 1, 1),
            a_few(5385, 14, 1, 1),
        ],
        _ => Vec::new(),
    }
}

/// The drops the game rolls differently depending on the world's mode.
///
/// The game writes these as one rule with two or three branches and picks the branch at the
/// moment of the kill, so they cannot live in the flat table: the same NPC drops different
/// amounts at different rates in classic, expert and master.
///
/// Only the branches that are plain rolls are here. Several of these rules have a branch that is
/// a treasure bag, a relic or a "one of these" draw, and those need machinery of their own.
fn by_mode(npc_type: u16, at: Conditions) -> Vec<Conditional> {
    /// Pick the branch the world is in: classic, expert, master.
    fn pick(
        at: Conditions,
        classic: Conditional,
        expert: Conditional,
        master: Conditional,
    ) -> Conditional {
        if at.master {
            master
        } else if at.expert {
            expert
        } else {
            classic
        }
    }

    match npc_type {
        // The Eater of Worlds: every segment. Shadow scales and demonite are what the whole
        // shadow-armour branch of progression is made of, and without them it is unreachable.
        13..=15 => vec![
            pick(
                at,
                a_few(86, 2, 1, 2),
                a_few(86, 5, 1, 2),
                a_few(86, 10, 1, 2),
            ),
            pick(
                at,
                a_few(56, 2, 2, 5),
                a_few(56, 2, 1, 3),
                a_few(56, 3, 1, 2),
            ),
        ],
        // King Slime's gel, and the Blue Slime's.
        326 => vec![pick(
            at,
            a_few(1729, 1, 1, 3),
            a_few(1729, 1, 1, 4),
            a_few(1729, 1, 2, 4),
        )],
        325 => vec![pick(
            at,
            a_few(1729, 1, 15, 30),
            a_few(1729, 1, 25, 40),
            a_few(1729, 1, 30, 50),
        )],
        // The Brain of Cthulhu: tissue samples and crimtane.
        266 => vec![
            pick(
                at,
                a_few(1329, 3, 2, 5),
                a_few(1329, 3, 1, 3),
                a_few(1329, 4, 1, 2),
            ),
            pick(
                at,
                a_few(880, 3, 5, 12),
                a_few(880, 3, 5, 7),
                a_few(880, 3, 2, 4),
            ),
        ],
        // A wyvern's souls of flight, which are more generous in expert.
        87 => vec![if at.expert {
            a_few(575, 1, 10, 20)
        } else {
            a_few(575, 1, 5, 10)
        }],
        // The Dungeon Guardian's bone key.
        185 => vec![if at.expert {
            a_few(5070, 1, 1, 3)
        } else {
            a_few(5070, 1, 1, 2)
        }],
        // Giant worms and their kin: the whoopie cushion.
        10 | 11 | 12 | 95 | 96 | 97 => vec![sometimes(215, 50)],
        // Hornets and their variants: the stinger.
        42 | 231 | 232 | 233 | 234 | 235 => vec![if at.expert {
            always(209)
        } else {
            sometimes(209, 3)
        }],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn plain() -> Conditions {
        Conditions::default()
    }

    /// A boss drops a bag in expert and not otherwise.
    #[test]
    fn expert_turns_a_boss_into_a_bag() {
        let classic = conditional(4, plain());
        assert!(
            !classic.iter().any(|d| d.item == 3319),
            "a bag in classic mode"
        );

        let expert = conditional(
            4,
            Conditions {
                expert: true,
                ..plain()
            },
        );
        assert!(expert.iter().any(|d| d.item == 3319 && d.one_in == 1));
    }

    /// Every bag and trophy is a real item, and no two bosses share one.
    #[test]
    fn the_bags_and_trophies_are_unique() {
        let mut bags = HashSet::new();
        let mut trophies = HashSet::new();
        for npc_type in 0..700u16 {
            if let Some(bag) = treasure_bag(npc_type) {
                // Two segments of the same worm share a bag, and the two eyes of Skeletron do too.
                bags.insert(bag);
            }
            if let Some(trophy) = trophy(npc_type) {
                trophies.insert(trophy);
            }
        }
        assert_eq!(bags.len(), 15, "fifteen bosses have bags: {bags:?}");
        assert_eq!(trophies.len(), 16, "and sixteen have trophies");
        // The Twins are the one boss whose halves have different trophies.
        assert_ne!(trophy(125), trophy(126));
        // ...but they share a bag.
        assert_eq!(treasure_bag(125), treasure_bag(126));
    }

    /// Hardmode materials do not drop before hardmode.
    #[test]
    fn hardmode_materials_wait_for_hardmode() {
        let underground_evil = Conditions {
            underground: true,
            in_corruption: true,
            ..plain()
        };
        assert!(
            conditional(3, underground_evil).is_empty(),
            "souls before the wall fell"
        );

        let after = Conditions {
            hard_mode: true,
            ..underground_evil
        };
        assert!(
            conditional(3, after).iter().any(|d| d.item == 547),
            "and none after"
        );
    }

    /// The souls are biome-specific: the evil's soul does not drop in the hallow.
    #[test]
    fn each_soul_keeps_to_its_own_biome() {
        let hallow = Conditions {
            hard_mode: true,
            underground: true,
            in_hallow: true,
            ..plain()
        };
        let drops: HashSet<u16> = conditional(3, hallow).iter().map(|d| d.item).collect();
        assert!(drops.contains(&548), "Soul of Light");
        assert!(!drops.contains(&547), "but not Soul of Night");
    }

    /// A soul needs depth as well as a biome.
    #[test]
    fn souls_need_depth() {
        let surface = Conditions {
            hard_mode: true,
            in_corruption: true,
            ..plain()
        };
        assert!(conditional(3, surface).is_empty(), "souls on the surface");
    }

    /// Plantera's key only comes once she is down.
    #[test]
    fn the_temple_key_waits_for_plantera() {
        let first = Conditions {
            hard_mode: true,
            ..plain()
        };
        assert!(!conditional(262, first).iter().any(|d| d.item == 1293));
        let after = Conditions {
            downed_plantera: true,
            ..first
        };
        assert!(conditional(262, after).iter().any(|d| d.item == 1293));
    }

    /// An ordinary enemy drops nothing conditional at all.
    #[test]
    fn most_things_drop_nothing_conditional() {
        let everything = Conditions {
            expert: true,
            master: false,
            hard_mode: true,
            downed_plantera: true,
            in_hallow: false,
            in_corruption: false,
            in_crimson: false,
            underground: false,
        };
        // A bunny, a goldfish, a guide.
        for ordinary in [46u16, 1, 22] {
            assert!(
                conditional(ordinary, everything).is_empty(),
                "{ordinary} dropped something conditional"
            );
        }
    }
}
