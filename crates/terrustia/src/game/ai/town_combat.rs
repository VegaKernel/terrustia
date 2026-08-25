//! Town NPCs fighting back.
//!
//! Vanilla drives every combat-capable town NPC through one function, `AI_007_TownEntities`
//! (`NPC.cs:53515-56130`), keyed by `NPCID.Sets.AttackType[type]` (`NPCID.cs:4855`): 0 and 1 and 2
//! are ranged (a projectile aimed at the target, states 10/12/14), 3 is melee (a hitbox swung
//! against the target via `StrikeNPCNoInteraction`, state 15). Of the ~27 town NPCs vanilla gives
//! an `AttackType`, this transcribes one representative of each of the four classes — enough to
//! prove the mechanism end to end and to make a town survive contact — rather than all 27 at once:
//!
//! * **Merchant** (17, type 0): a pistol shot, `NPC.cs:54969` (`type == 17` branch of state 10).
//! * **Arms Dealer** (19, type 1): a musket shot, `NPC.cs:55114` (state 12, non-hardmode values —
//!   the hardmode burst-fire upgrade is not modelled).
//! * **Wizard** (108, type 2): a bolt, `NPC.cs:55428` (state 14).
//! * **Dye Trader** (207, type 3): a melee swing, `NPC.cs:55611-55637` (state 15) — the only one of
//!   the four that does not fire a projectile at all: it builds a hitbox and calls
//!   `StrikeNPCNoInteraction` directly on anything it intersects.
//!
//! **This is a reimplementation of each class's core behaviour, not a line-by-line port** — the
//! same standing distinction `npc_ai.rs`'s module doc draws for every other AI style. Specifically
//! not modelled: the multi-frame windup before the shot actually leaves (vanilla holds `ai[0]` in
//! the attack state for `AttackTime[type]` ticks first; this fires as soon as the decision is
//! made), the exact probabilistic `AttackAverageChance` gate (approximated as a flat cooldown), and
//! the vertical aim-tolerance check ranged types 1 and 2 use to decide whether to even attempt a
//! shot. None of these change *whether* a town NPC fights back or *what it hits with* — only the
//! precise cadence — which is the distinction this module's own reimplementation stance exists for.
//!
//! Adding the remaining ~23 town NPCs is mechanical from here: read their branch in the same
//! function, add an entry to [`town_combat`]. Nothing about the framework below is specific to
//! these four.

/// What one town NPC type's attack looks like.
#[derive(Debug, Clone, Copy)]
pub struct TownCombat {
    /// Vanilla's own `ai[0]` value for this attack class (`NPC.cs`'s state 10/12/14/15) — kept so
    /// a real client's own animation prediction, which reads this field for any NPC, has a state
    /// it recognises rather than a number invented for this port.
    pub state: f32,
    pub kind: AttackKind,
    /// How far a hostile has to be before this NPC notices it, from `NPCID.Sets.DangerDetectRange`.
    pub range: f32,
    /// Ticks between attacks. Approximates vanilla's `AttackTime[type]` windup plus an
    /// `AttackAverageChance[type]`-driven random gate as one flat number, per the module doc.
    pub cooldown: i32,
}

#[derive(Debug, Clone, Copy)]
pub enum AttackKind {
    Ranged {
        projectile: u16,
        /// Vanilla's own pre-scaling damage — run it through [`town_npc_damage`] before use.
        damage: i32,
        speed: f32,
        knockback: f32,
    },
    Melee {
        damage: i32,
        knockback: f32,
        /// Half-width and half-height of the swing's hitbox, centred on the NPC.
        reach: (f32, f32),
    },
}

/// The four representative town NPCs this module covers. `None` for everything else — including
/// every other real vanilla `AttackType` NPC — is correct today, not a gap this function hides:
/// those NPCs simply do not fight yet, the same as before this module existed.
pub fn town_combat(npc_type: u16) -> Option<TownCombat> {
    Some(match npc_type {
        // Merchant. NPC.cs:54969-54977: projectile 48, speed 9, damage 12, knockback 1.5.
        // AttackTime[17]=40, AttackAverageChance[17]=30, DangerDetectRange[17]=320.
        17 => TownCombat {
            state: 10.0,
            kind: AttackKind::Ranged {
                projectile: 48,
                damage: 12,
                speed: 9.0,
                knockback: 1.5,
            },
            range: 320.0,
            cooldown: 70,
        },
        // Arms Dealer. NPC.cs:55114-55123, non-hardmode: projectile 14, speed 13, damage 24,
        // knockback 3. AttackTime[19]=40, AttackAverageChance[19]=30, DangerDetectRange[19]=900.
        19 => TownCombat {
            state: 12.0,
            kind: AttackKind::Ranged {
                projectile: 14,
                damage: 24,
                speed: 13.0,
                knockback: 3.0,
            },
            range: 900.0,
            cooldown: 70,
        },
        // Wizard. NPC.cs:55428-55438: projectile 15, speed 6, damage 18, knockback 3.
        // AttackTime[108]=60, AttackAverageChance[108]=30, DangerDetectRange[108]=700.
        108 => TownCombat {
            state: 14.0,
            kind: AttackKind::Ranged {
                projectile: 15,
                damage: 18,
                speed: 6.0,
                knockback: 3.0,
            },
            range: 700.0,
            cooldown: 90,
        },
        // Dye Trader. NPC.cs:55611-55617: damage 11, knockback 4.25, hitbox 32x32. The "Andrew"
        // easter egg (NPC.cs:55618, a doubled Tax Collector) is deliberately not carried over —
        // it is cosmetic flavour tied to a specific `GivenName`, not a gameplay gap.
        207 => TownCombat {
            state: 15.0,
            kind: AttackKind::Melee {
                damage: 11,
                knockback: 4.25,
                reach: (32.0, 32.0),
            },
            range: 60.0,
            cooldown: 24,
        },
        _ => return None,
    })
}

/// `NPC.GetAttackDamage_ForTownNPC`, transcribed: `GameDifficultyData.TownNPCDamageMultiplier` is
/// 1.0 in classic and 1.5 in expert-or-better. Journey's 2.0 and master's separate 2.0 are not
/// modelled — this project has no Journey-mode power state yet (`FEATURES.md`), and `Conditions`
/// carries only `expert`, not a distinct master flag.
pub fn town_npc_damage(base: i32, expert: bool) -> i32 {
    let multiplier = if expert { 1.5 } else { 1.0 };
    ((base as f32) * multiplier) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_representative_npcs_are_covered() {
        for (npc_type, expect_melee) in [(17, false), (19, false), (108, false), (207, true)] {
            let combat = town_combat(npc_type).unwrap_or_else(|| panic!("npc {npc_type}"));
            assert_eq!(
                matches!(combat.kind, AttackKind::Melee { .. }),
                expect_melee
            );
            assert!(combat.range > 0.0);
            assert!(combat.cooldown > 0);
        }
    }

    #[test]
    fn everything_else_still_does_not_fight() {
        // The Guide (22) is a real vanilla AttackType-1 NPC, deliberately not one of the four this
        // pass covers — its absence here must stay a `None`, not silently become a shot.
        assert!(town_combat(22).is_none());
        assert!(town_combat(1).is_none(), "a blue slime is not a town NPC");
    }

    #[test]
    fn expert_mode_scales_damage_by_one_and_a_half() {
        assert_eq!(town_npc_damage(12, false), 12);
        assert_eq!(town_npc_damage(12, true), 18);
    }
}
