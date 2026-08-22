//! What touching an enemy does to you beyond the damage.
//!
//! A little over half the roster leaves something behind. Some of it is flavour — a jellyfish
//! electrifies you, a bat gives you Feral Bite — and some of it is the actual difficulty of a
//! biome: the corruption's Weakness and Slow are why fighting there at low level is harder than the
//! damage numbers suggest, and the Ichor a Crimson enemy leaves cuts your armour outright.
//!
//! Several of these only exist in expert. That is not a damage multiplier: a boss that inflicts a
//! debuff in expert and none in classic is a different fight, not a harder one.

/// A debuff an enemy can land by touching you.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Touch {
    /// The buff id.
    pub buff: u16,
    /// A one-in-this chance. One means always.
    pub one_in: u32,
    /// How long it lasts, in ticks, as a range.
    pub ticks: (i32, i32),
    /// Whether it only happens in expert mode or above.
    pub expert_only: bool,
}

const fn touch(buff: u16, one_in: u32, from: i32, to: i32) -> Touch {
    Touch {
        buff,
        one_in,
        ticks: (from, to),
        expert_only: false,
    }
}

const fn expert(buff: u16, one_in: u32, from: i32, to: i32) -> Touch {
    Touch {
        buff,
        one_in,
        ticks: (from, to),
        expert_only: true,
    }
}

const POISONED_IN_EXPERT: [Touch; 1] = [expert(20, 1, 60, 240)];
const POISONED_IN_EXPERT_2: [Touch; 1] = [expert(20, 1, 60, 180)];
const POISONED: [Touch; 1] = [touch(20, 2, 600, 600)];
const BLEEDING: [Touch; 1] = [touch(23, 3, 240, 240)];
const CURSED: [Touch; 1] = [touch(36, 6, 7200, 7200)];
const SLOW: [Touch; 1] = [touch(30, 5, 2700, 2700)];
const SLOW_2: [Touch; 1] = [touch(30, 1, 600, 1200)];
const SLOW_IN_EXPERT: [Touch; 1] = [expert(30, 1, 360, 600)];
const WITHERED_ARMOUR_AND_WEAK: [Touch; 2] = [touch(35, 10, 420, 420), touch(32, 8, 900, 900)];
const WITHERED_ARMOUR: [Touch; 1] = [touch(35, 5, 420, 420)];
const WEAK: [Touch; 1] = [touch(32, 8, 900, 900)];
const WEAK_IN_EXPERT: [Touch; 1] = [expert(32, 2, 30, 60)];
const BROKEN_ARMOUR: [Touch; 1] = [touch(33, 20, 18000, 18000)];
const BROKEN_ARMOUR_2: [Touch; 1] = [touch(33, 25, 7200, 7200)];
const DARKNESS: [Touch; 1] = [touch(22, 4, 900, 900)];
const SILENCED: [Touch; 1] = [touch(31, 14, 300, 300)];
const SILENCED_2: [Touch; 1] = [touch(31, 1, 840, 840)];
const CONFUSED: [Touch; 1] = [touch(24, 3, 420, 420)];
const CURSED_INFERNO: [Touch; 1] = [touch(39, 1, 240, 240)];
const CHILLED: [Touch; 1] = [touch(46, 12, 600, 600)];
const CHILLED_AND_FROZEN: [Touch; 2] = [touch(46, 2, 900, 900), touch(47, 35, 60, 60)];
const CHILLED_AND_FROZEN_2: [Touch; 2] = [touch(46, 1, 1200, 1200), touch(47, 15, 60, 60)];
const VENOM: [Touch; 1] = [touch(70, 10, 240, 240)];
const VENOM_2: [Touch; 1] = [touch(70, 1, 240, 241)];
const ICHOR: [Touch; 1] = [touch(69, 1, 420, 420)];
const FERAL_BITE_IN_EXPERT: [Touch; 1] = [expert(148, 10, 1800, 5400)];
const SHADOWFLAME: [Touch; 1] = [touch(103, 1, 180, 480)];
const CURSED_2: [Touch; 1] = [touch(36, 2, 600, 600)];
const BROKEN_ARMOUR_3: [Touch; 1] = [touch(33, 10, 3600, 3600)];
const CONFUSED_2: [Touch; 1] = [touch(24, 1, 600, 600)];
const SLOW_AND_WEAK: [Touch; 2] = [touch(30, 3, 1200, 1200), touch(32, 3, 300, 300)];

/// Every rule in the table: which types it applies to, and what it leaves.
///
/// A flat list rather than a match, because the game rolls every rule an enemy matches
/// rather than the first: a Dark Caster leaves both Darkness and Weakness, and a match arm
/// would silently drop the second.
const RULES: &[(&[u16], &[Touch])] = &[
    (&[222], &POISONED_IN_EXPERT),
    (&[210, 211], &POISONED_IN_EXPERT_2),
    (&[141], &POISONED),
    (&[34, 83, 84, 179, 289], &BLEEDING),
    (&[77], &CURSED),
    (&[273, 274, 275, 276], &CURSED_2),
    (&[104, 102], &SLOW),
    (&[158, 159], &SLOW_2),
    (&[35], &SLOW_IN_EXPERT),
    (&[75], &WITHERED_ARMOUR_AND_WEAK),
    (&[79, 103, 630], &WITHERED_ARMOUR),
    (&[78, 82], &WEAK),
    (&[36], &WEAK_IN_EXPERT),
    (&[112], &BROKEN_ARMOUR),
    (&[182], &BROKEN_ARMOUR_2),
    (
        &[305, 306, 307, 308, 309, 310, 311, 312, 313, 314],
        &BROKEN_ARMOUR_3,
    ),
    (&[1, 79, 81, 183, 630], &DARKNESS),
    (&[93, 109, 80], &SILENCED),
    (&[527], &SILENCED_2),
    (&[23, 25], &CONFUSED),
    (&[277, 278, 279, 280], &CONFUSED_2),
    (&[525], &CURSED_INFERNO),
    (&[147], &CHILLED),
    (&[150], &CHILLED_AND_FROZEN),
    (&[184], &CHILLED_AND_FROZEN_2),
    (&[163, 236, 237, 238], &VENOM),
    (&[530, 531], &VENOM_2),
    (&[526], &ICHOR),
    (&[269, 270, 271, 272], &SLOW_AND_WEAK),
    (&[49, 51, 93, 152, 634], &FERAL_BITE_IN_EXPERT),
    (&[371], &SHADOWFLAME),
];

/// What an enemy leaves behind. Most leave nothing.
pub fn on_touch(npc_type: u16) -> impl Iterator<Item = Touch> {
    RULES
        .iter()
        .filter(move |(types, _)| types.contains(&npc_type))
        .flat_map(|(_, rules)| rules.iter().copied())
}

/// Whether an enemy leaves anything at all.
pub fn leaves_anything(npc_type: u16) -> bool {
    on_touch(npc_type).next().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is coherent: no zero chances, no backwards durations.
    #[test]
    fn every_rule_is_well_formed() {
        for npc_type in 0..700u16 {
            for rule in on_touch(npc_type) {
                assert!(rule.one_in >= 1, "{npc_type} has a zero chance");
                assert!(
                    rule.ticks.0 > 0 && rule.ticks.1 >= rule.ticks.0,
                    "{npc_type} has a backwards duration {:?}",
                    rule.ticks
                );
                assert!(rule.buff > 0, "{npc_type} names buff zero");
            }
        }
    }

    /// Everything named is a type this build has.
    #[test]
    fn only_real_enemies_are_named() {
        for npc_type in 0..700u16 {
            if leaves_anything(npc_type) {
                assert!(
                    crate::npc_data::npc_stats(npc_type).is_some(),
                    "{npc_type} leaves a debuff but is not a type this build has"
                );
            }
        }
    }

    /// The expert-only rules really are gated.
    #[test]
    fn expert_rules_are_marked() {
        // Queen Bee's poison is expert-only; a hornet's is not.
        assert!(on_touch(222).all(|r| r.expert_only));
        assert!(on_touch(141).all(|r| !r.expert_only));
    }

    /// An enemy can leave two things, and a Frost Legion soldier is the plainest case.
    #[test]
    fn some_enemies_leave_two_things() {
        let snowman: Vec<Touch> = on_touch(184).collect();
        assert_eq!(snowman.len(), 2, "chilled and frozen");
        let buffs: Vec<u16> = snowman.iter().map(|r| r.buff).collect();
        assert_eq!(buffs, vec![46, 47]);
        // Chilled is certain, frozen is not.
        assert_eq!(snowman[0].one_in, 1);
        assert!(snowman[1].one_in > 1);
    }

    /// An enemy matching two rules gets both, not the first.
    ///
    /// A match arm would have silently dropped the second, and did until the compiler pointed at
    /// the unreachable pattern: a Dark Caster is in both the Withered Armour list and the Darkness
    /// one, and was only ever landing the first.
    #[test]
    fn an_enemy_in_two_rules_lands_both() {
        let caster: Vec<u16> = on_touch(79).map(|r| r.buff).collect();
        assert!(caster.contains(&35), "Withered Armour");
        assert!(caster.contains(&22), "and Darkness");

        let mage: Vec<u16> = on_touch(93).map(|r| r.buff).collect();
        assert!(mage.contains(&31), "Silenced");
        assert!(mage.contains(&148), "and Feral Bite in expert");
    }

    /// Most of the roster leaves nothing, which is what makes the ones that do worth noticing.
    #[test]
    fn most_enemies_leave_nothing() {
        let leaving = (0..700u16).filter(|t| leaves_anything(*t)).count();
        assert!(leaving > 30, "only {leaving} enemies debuff you");
        assert!(leaving < 200, "{leaving} is most of the roster");
    }
}
