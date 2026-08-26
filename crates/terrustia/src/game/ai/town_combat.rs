//! Town NPCs fighting back.
//!
//! Vanilla drives every combat-capable town NPC through one function, `AI_007_TownEntities`
//! (`NPC.cs:53515-56130`), keyed by `NPCID.Sets.AttackType[type]` (`NPCID.cs:4855`): 0 and 1 and 2
//! are ranged (a projectile aimed at the target, states 10/12/14 respectively), 3 is melee (a
//! hitbox swung against the target via `StrikeNPCNoInteraction`, state 15). All 28 town NPCs
//! vanilla gives an `AttackType` are transcribed here — the first four (Merchant/Arms
//! Dealer/Wizard/Dye Trader) proved the mechanism end to end earlier this session; this pass is
//! the rest.
//!
//! **This is a reimplementation of each class's core behaviour, not a line-by-line port** — the
//! same standing distinction `npc_ai.rs`'s module doc draws for every other AI style. Specifically
//! not modelled, for every entry: the multi-frame windup before the shot actually leaves (vanilla
//! holds `ai[0]` in the attack state for `AttackTime[type]` ticks first; this fires as soon as the
//! decision is made), the exact probabilistic `AttackAverageChance` gate (approximated below as a
//! flat `cooldown` of `AttackTime[type] + AttackAverageChance[type]` — exact for Arms Dealer,
//! close for everything else, since vanilla's own gate is itself a per-tick geometric roll this
//! project has no equivalent scheduling primitive for), and the vertical aim-tolerance check the
//! ranged classes use to decide whether to even attempt a shot. None of these change *whether* a
//! town NPC fights back or *what it hits with* — only the precise cadence.
//!
//! A handful of NPCs needed a further, per-entry simplification beyond that flat list — each is
//! called out at its own entry below, not silently folded into the general disclaimer:
//! - **Hardmode upgrades are never modelled** (Arms Dealer's burst fire, Guide's homing bolt,
//!   Steampunker's/Travelling Merchant's/Painter's/Pirate's damage or projectile changes,
//!   Princess's higher damage) — every entry here uses vanilla's classic-mode values, matching the
//!   precedent Arms Dealer's own entry already set.
//! - **Pirate's escalating multi-shot burst and close-range special attack are not modelled** —
//!   only its base single shot (`NPC.cs`, `type == 229` branch's initial values before the
//!   `localAI[3] > num54` escalation ladder and the `PrettySafe`-range special case).
//! - **Cyborg picks one of three random projectiles per shot in vanilla** (`Utils.SelectRandom`
//!   among rocket/grenade/proximity-mine launchers); this always fires the rocket launcher variant
//!   (case `135`) rather than modelling the roll.
//! - **Truffle and Princess do not throw their projectile from themselves** — vanilla spawns it at
//!   a position near the target instead (a mushroom sprouting, a heart projectile appearing), with
//!   no meaningful launch velocity. Modelled here as an ordinary aimed shot like every other ranged
//!   entry, since this module's `AttackKind::Ranged` has no "spawn near target" shape and adding
//!   one for two NPCs was judged not worth the complexity — both still deal real damage on a real
//!   cooldown, which is what matters for "the town fights back."
//! - **Dryad's ranged attack does zero pre-scaling damage in vanilla** (`NPC.cs`'s `type == 20`
//!   branch never sets the damage local the way every other branch does, leaving it at its
//!   declared-zero default) — transcribed faithfully rather than "corrected," the same standing
//!   rule this session's other genuinely-dead-vanilla-branch transcriptions already follow. Her
//!   `AttackTime[20]` is also a real outlier at 600 (vs. 15-90 for everyone else), so she attacks
//!   far less often than the rest even before that. The net effect — a rare, harmless shot — reads
//!   as intentional in vanilla too (a nature spirit's projectile is closer to a visual effect than
//!   a weapon).
//! - **Tax Collector's "Andrew" easter egg** (`GivenName == "Andrew"`, a doubled Tax Collector) is
//!   deliberately not carried over, the same call already made for Dye Trader's own easter egg —
//!   cosmetic flavour tied to a specific name, not a gameplay gap.

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

/// Every vanilla town NPC that has a real `AttackType`. `None` for everything else — town pets
/// (`NPCID.Sets.IsTownPet`, explicitly re-asserted as `-1` in vanilla's own `AttackType` set) and
/// anything without an `AttackType` entry at all — is correct, not a gap this function hides.
pub fn town_combat(npc_type: u16) -> Option<TownCombat> {
    Some(match npc_type {
        // ---- AttackType 0, state 10: ranged, aimed from the NPC toward the target's head ----
        // Demolitionist. NPC.cs state-10 block, `type == 38`: projectile 30, damage 20, speed 6,
        // knockback 7. AttackTime[38]=34, AttackAverageChance[38]=40, DangerDetectRange[38]=300.
        38 => TownCombat {
            state: 10.0,
            kind: AttackKind::Ranged {
                projectile: 30,
                damage: 20,
                speed: 6.0,
                knockback: 7.0,
            },
            range: 300.0,
            cooldown: 74,
        },
        // Bestiary Girl. NPC.cs state-10 block, `type == 633`: projectile 880, damage 15, speed 24,
        // knockback 7 (the "lycantrope" full-moon variant, projectile 929 with 1.5x damage, is not
        // modelled — a calendar-gated cosmetic swap, same call as every other secret-seed/date-gated
        // branch this project skips). AttackTime[633]=12, AttackAverageChance[633]=1,
        // DangerDetectRange[633]=100.
        633 => TownCombat {
            state: 10.0,
            kind: AttackKind::Ranged {
                projectile: 880,
                damage: 15,
                speed: 24.0,
                knockback: 7.0,
            },
            range: 100.0,
            cooldown: 13,
        },
        // DD2 Bartender (Tavernkeep). NPC.cs state-10 block, `type == 550`: projectile 669, damage
        // 24, speed 6, knockback 9. AttackTime[550]=34, AttackAverageChance[550]=40,
        // DangerDetectRange[550]=120.
        550 => TownCombat {
            state: 10.0,
            kind: AttackKind::Ranged {
                projectile: 669,
                damage: 24,
                speed: 6.0,
                knockback: 9.0,
            },
            range: 120.0,
            cooldown: 74,
        },
        // Golfer. NPC.cs state-10 block, `type == 588`: projectile 721, damage 15, speed 8,
        // knockback 9. AttackTime[588]=20, AttackAverageChance[588]=20,
        // DangerDetectRange[588]=120.
        588 => TownCombat {
            state: 10.0,
            kind: AttackKind::Ranged {
                projectile: 721,
                damage: 15,
                speed: 8.0,
                knockback: 9.0,
            },
            range: 120.0,
            cooldown: 40,
        },
        // Party Girl. NPC.cs state-10 block, `type == 208`: projectile 588, damage 30, speed 6,
        // knockback 6. AttackTime[208]=34, AttackAverageChance[208]=50,
        // DangerDetectRange[208]=400.
        208 => TownCombat {
            state: 10.0,
            kind: AttackKind::Ranged {
                projectile: 588,
                damage: 30,
                speed: 6.0,
                knockback: 6.0,
            },
            range: 400.0,
            cooldown: 84,
        },
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
        // Angler. NPC.cs state-10 block, `type == 369`: projectile 520, damage 10, speed 12,
        // knockback 3. AttackTime[369]=34, AttackAverageChance[369]=50,
        // DangerDetectRange[369]=300.
        369 => TownCombat {
            state: 10.0,
            kind: AttackKind::Ranged {
                projectile: 520,
                damage: 10,
                speed: 12.0,
                knockback: 3.0,
            },
            range: 300.0,
            cooldown: 84,
        },
        // Skeleton Merchant. NPC.cs state-10 block, `type == 453`: projectile 21, damage 14, speed
        // 14, knockback 3. AttackTime[453]=34, AttackAverageChance[453]=30,
        // DangerDetectRange[453]=300.
        453 => TownCombat {
            state: 10.0,
            kind: AttackKind::Ranged {
                projectile: 21,
                damage: 14,
                speed: 14.0,
                knockback: 3.0,
            },
            range: 300.0,
            cooldown: 64,
        },
        // Goblin Tinkerer. NPC.cs state-10 block, `type == 107`: projectile 24, damage 15, speed 5,
        // knockback 1. AttackTime[107]=60, AttackAverageChance[107]=60,
        // DangerDetectRange[107]=300.
        107 => TownCombat {
            state: 10.0,
            kind: AttackKind::Ranged {
                projectile: 24,
                damage: 15,
                speed: 5.0,
                knockback: 1.0,
            },
            range: 300.0,
            cooldown: 120,
        },
        // Mechanic. NPC.cs state-10 block, `type == 124`: projectile 582, damage 11, speed 10,
        // knockback 3.5. AttackTime[124]=34, AttackAverageChance[124]=30,
        // DangerDetectRange[124]=800.
        124 => TownCombat {
            state: 10.0,
            kind: AttackKind::Ranged {
                projectile: 582,
                damage: 11,
                speed: 10.0,
                knockback: 3.5,
            },
            range: 800.0,
            cooldown: 64,
        },
        // Nurse. NPC.cs state-10 block, `type == 18`: projectile 583, damage 8, speed 8, knockback
        // 2. AttackTime[18]=34, AttackAverageChance[18]=60, DangerDetectRange[18]=300.
        18 => TownCombat {
            state: 10.0,
            kind: AttackKind::Ranged {
                projectile: 583,
                damage: 8,
                speed: 8.0,
                knockback: 2.0,
            },
            range: 300.0,
            cooldown: 94,
        },
        // Santa Claus. NPC.cs state-10 block, `type == 142`: projectile 589, damage 22, speed 7,
        // knockback 2. AttackTime[142]=34, AttackAverageChance[142]=50,
        // DangerDetectRange[142]=500.
        142 => TownCombat {
            state: 10.0,
            kind: AttackKind::Ranged {
                projectile: 589,
                damage: 22,
                speed: 7.0,
                knockback: 2.0,
            },
            range: 500.0,
            cooldown: 84,
        },

        // ---- AttackType 1, state 12: ranged, aimed at the target's centre ----
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
        // Painter. NPC.cs state-12 block, `type == 227`, non-hardmode: projectile 587, damage 8,
        // speed 10, knockback 1.75. AttackTime[227]=60, AttackAverageChance[227]=30,
        // DangerDetectRange[227]=800.
        227 => TownCombat {
            state: 12.0,
            kind: AttackKind::Ranged {
                projectile: 587,
                damage: 8,
                speed: 10.0,
                knockback: 1.75,
            },
            range: 800.0,
            cooldown: 90,
        },
        // Travelling Merchant. NPC.cs state-12 block, `type == 368`, non-hardmode: projectile 14,
        // damage 24, speed 13, knockback 2. AttackTime[368]=60, AttackAverageChance[368]=40,
        // DangerDetectRange[368]=900.
        368 => TownCombat {
            state: 12.0,
            kind: AttackKind::Ranged {
                projectile: 14,
                damage: 24,
                speed: 13.0,
                knockback: 2.0,
            },
            range: 900.0,
            cooldown: 100,
        },
        // Guide. NPC.cs state-12 block, `type == 22`, non-hardmode: projectile 1, damage 12, speed
        // 10, knockback 2.75. AttackTime[22]=30, AttackAverageChance[22]=30,
        // DangerDetectRange[22]=700.
        22 => TownCombat {
            state: 12.0,
            kind: AttackKind::Ranged {
                projectile: 1,
                damage: 12,
                speed: 10.0,
                knockback: 2.75,
            },
            range: 700.0,
            cooldown: 60,
        },
        // Witch Doctor. NPC.cs state-12 block, `type == 228`: projectile 267, damage 20, speed 14,
        // knockback 3. AttackTime[228]=40, AttackAverageChance[228]=50,
        // DangerDetectRange[228]=800.
        228 => TownCombat {
            state: 12.0,
            kind: AttackKind::Ranged {
                projectile: 267,
                damage: 20,
                speed: 14.0,
                knockback: 3.0,
            },
            range: 800.0,
            cooldown: 90,
        },
        // Steampunker. NPC.cs state-12 block, `type == 178`, non-hardmode: projectile 242, damage
        // 11, speed 13, knockback 2. AttackTime[178]=24, AttackAverageChance[178]=50,
        // DangerDetectRange[178]=900.
        178 => TownCombat {
            state: 12.0,
            kind: AttackKind::Ranged {
                projectile: 242,
                damage: 11,
                speed: 13.0,
                knockback: 2.0,
            },
            range: 900.0,
            cooldown: 74,
        },
        // Pirate. NPC.cs state-12 block, `type == 229`, base shot only (see module doc — the
        // escalating burst and close-range special are not modelled): projectile 14, damage 24,
        // speed 14, knockback 2. AttackTime[229]=60, AttackAverageChance[229]=40,
        // DangerDetectRange[229]=1000.
        229 => TownCombat {
            state: 12.0,
            kind: AttackKind::Ranged {
                projectile: 14,
                damage: 24,
                speed: 14.0,
                knockback: 2.0,
            },
            range: 1000.0,
            cooldown: 100,
        },
        // Cyborg. NPC.cs state-12 block, `type == 209`, `case 135` only (see module doc — vanilla
        // picks one of three projectiles per shot): projectile 135, damage 30, speed 12, knockback
        // 7. AttackTime[209]=60, AttackAverageChance[209]=30, DangerDetectRange[209]=1000.
        209 => TownCombat {
            state: 12.0,
            kind: AttackKind::Ranged {
                projectile: 135,
                damage: 30,
                speed: 12.0,
                knockback: 7.0,
            },
            range: 1000.0,
            cooldown: 90,
        },

        // ---- AttackType 2, state 14: ranged, aimed with a slight downward lead ----
        // Clothier. NPC.cs state-14 block, `type == 54`: projectile 585, damage 16, speed 10,
        // knockback 2. AttackTime[54]=60, AttackAverageChance[54]=30,
        // DangerDetectRange[54]=700.
        54 => TownCombat {
            state: 14.0,
            kind: AttackKind::Ranged {
                projectile: 585,
                damage: 16,
                speed: 10.0,
                knockback: 2.0,
            },
            range: 700.0,
            cooldown: 90,
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
        // Truffle. NPC.cs state-14 block, `type == 160` — spawns near the target rather than being
        // thrown, see module doc: projectile 590, damage 40, speed approximated at 6 (vanilla has
        // no real launch velocity here), knockback 3. AttackTime[160]=60,
        // AttackAverageChance[160]=60, DangerDetectRange[160]=700.
        160 => TownCombat {
            state: 14.0,
            kind: AttackKind::Ranged {
                projectile: 590,
                damage: 40,
                speed: 6.0,
                knockback: 3.0,
            },
            range: 700.0,
            cooldown: 120,
        },
        // Princess. NPC.cs state-14 block, `type == 663`, non-hardmode — spawns near the target
        // rather than being thrown, see module doc: projectile 950, damage 15, speed approximated
        // at 6, knockback 3. AttackTime[663]=60, AttackAverageChance[663]=1,
        // DangerDetectRange[663]=700.
        663 => TownCombat {
            state: 14.0,
            kind: AttackKind::Ranged {
                projectile: 950,
                damage: 15,
                speed: 6.0,
                knockback: 3.0,
            },
            range: 700.0,
            cooldown: 61,
        },
        // Dryad. NPC.cs state-14 block, `type == 20` — real vanilla zero-damage attack, see module
        // doc: projectile 586, damage 0, speed approximated at 6, knockback 3.
        // AttackTime[20]=600, AttackAverageChance[20]=60, DangerDetectRange[20]=1200.
        20 => TownCombat {
            state: 14.0,
            kind: AttackKind::Ranged {
                projectile: 586,
                damage: 0,
                speed: 6.0,
                knockback: 3.0,
            },
            range: 1200.0,
            cooldown: 660,
        },

        // ---- AttackType 3, state 15: melee, a hitbox swung at anything it overlaps ----
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
        // Tax Collector. NPC.cs state-15 block, `type == 441`: damage 9, knockback 3.5, hitbox
        // 28x28 (the "Andrew" easter egg, see module doc, not carried over). AttackTime[441]=15,
        // AttackAverageChance[441]=1, DangerDetectRange[441]=50.
        441 => TownCombat {
            state: 15.0,
            kind: AttackKind::Melee {
                damage: 9,
                knockback: 3.5,
                reach: (28.0, 28.0),
            },
            range: 50.0,
            cooldown: 16,
        },
        // Stylist. NPC.cs state-15 block, `type == 353`: damage 15, knockback 5, hitbox 32x32.
        // AttackTime[353]=12, AttackAverageChance[353]=1, DangerDetectRange[353]=60.
        353 => TownCombat {
            state: 15.0,
            kind: AttackKind::Melee {
                damage: 15,
                knockback: 5.0,
                reach: (32.0, 32.0),
            },
            range: 60.0,
            cooldown: 13,
        },
        _ => return None,
    })
}

/// `NPC.GetAttackDamage_ForTownNPC`, transcribed: `GameDifficultyData.TownNPCDamageMultiplier` is
/// 1.0 in classic and 1.5 in expert-or-better. Journey's 2.0 and master's separate 2.0 are not
/// modelled — this project has no Journey-mode power state yet (`README.md`), and `Conditions`
/// carries only `expert`, not a distinct master flag.
pub fn town_npc_damage(base: i32, expert: bool) -> i32 {
    let multiplier = if expert { 1.5 } else { 1.0 };
    ((base as f32) * multiplier) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every real vanilla `AttackType` NPC: 28 total, one per `(npc_type, is_melee)` pair from
    /// `NPCID.cs`'s own `AttackType` set (`Factory.CreateIntSet(-1, 38, 0, 17, 0, ...)`).
    const ALL_COMBAT_NPCS: [(u16, bool); 28] = [
        (38, false),
        (17, false),
        (107, false),
        (19, false),
        (22, false),
        (124, false),
        (228, false),
        (178, false),
        (18, false),
        (229, false),
        (209, false),
        (54, false),
        (108, false),
        (160, false),
        (20, false),
        (369, false),
        (453, false),
        (368, false),
        (207, true),
        (227, false),
        (208, false),
        (142, false),
        (441, true),
        (353, true),
        (633, false),
        (550, false),
        (588, false),
        (663, false),
    ];

    #[test]
    fn every_real_attack_type_npc_is_covered() {
        for (npc_type, expect_melee) in ALL_COMBAT_NPCS {
            let combat = town_combat(npc_type).unwrap_or_else(|| panic!("npc {npc_type}"));
            assert_eq!(
                matches!(combat.kind, AttackKind::Melee { .. }),
                expect_melee,
                "npc {npc_type}"
            );
            assert!(combat.range > 0.0, "npc {npc_type}");
            assert!(combat.cooldown > 0, "npc {npc_type}");
        }
    }

    #[test]
    fn no_two_covered_npcs_share_an_id() {
        let mut ids: Vec<u16> = ALL_COMBAT_NPCS.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        let mut deduped = ids.clone();
        deduped.dedup();
        assert_eq!(ids, deduped, "a duplicate id in the coverage table itself");
    }

    #[test]
    fn town_pets_still_do_not_fight() {
        // Town Cat/Dog/Bunny: vanilla's own `AttackType` set explicitly re-asserts `-1` for these
        // (`NPCID.Sets.IsTownPet`) rather than leaving them at the array's default — the same
        // "not a fighter" outcome, just spelled out in source rather than implied.
        for pet in [637u16, 638, 656] {
            assert!(town_combat(pet).is_none(), "npc {pet} is a town pet");
        }
        assert!(town_combat(1).is_none(), "a blue slime is not a town NPC");
    }

    #[test]
    fn expert_mode_scales_damage_by_one_and_a_half() {
        assert_eq!(town_npc_damage(12, false), 12);
        assert_eq!(town_npc_damage(12, true), 18);
    }

    #[test]
    fn dryads_attack_is_faithfully_harmless() {
        // Not a bug — see the module doc's own entry for why vanilla's `type == 20` branch never
        // sets a damage value.
        let combat = town_combat(20).unwrap();
        assert!(matches!(combat.kind, AttackKind::Ranged { damage: 0, .. }));
    }
}
