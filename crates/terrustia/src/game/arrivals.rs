//! Who moves into a town, and what they are waiting for.
//!
//! Only the Guide ever arrived before this. Everyone else waits on a condition, and with no
//! condition table there was nothing to check — so a world could have a street of empty houses and
//! one Guide in it forever. That is not a cosmetic gap: **no Mechanic means no wire**, and the
//! whole wiring system is implemented and was unreachable; no Goblin Tinkerer means no reforging;
//! no Merchant means no piggy bank; no Nurse means no healing.
//!
//! The conditions are `Main.cs`'s `townNPCCanSpawn` chain. Most are world progress, which the
//! server already knows. Two read what players are carrying, which the server sees because clients
//! sync their inventory — believed rather than verified, exactly as vanilla does.
//!
//! Eight of them wait on a *rescue* instead: the Goblin Tinkerer, Wizard, Mechanic, Stylist,
//! Angler, Tax Collector, Golfer and Tavernkeep are all found bound somewhere in the world and
//! freed. Those flags live on [`Progress`] and are read here, so a world loaded from Terraria with
//! them already rescued gets its residents. A world generated *here* cannot set them yet, because
//! nothing places the bound NPCs — recorded in GAPS.md rather than papered over.
//!
//! A handful of residents move in on state the server does not yet track, so their real condition
//! is transcribed (`Main.cs:66755-66940`) but cannot fire here and is disclosed rather than faked:
//! Santa needs the December calendar `Main.xMas` (no seasonal calendar exists server-side), the
//! Zoologist needs 10% bestiary completion (no bestiary tracking), the cat / dog / bunny pets need
//! a license bought from the Zoologist (`NPC.boughtCat/Dog/Bunny`, no shop-purchase tracking), and
//! the researched town slimes (Blue, Old, Purple, Rainbow, Red, Yellow, Copper) need Journey-mode
//! research unlocks. The ones the server *can* satisfy — Truffle, Party Girl, the party-time Green
//! Slime and the Princess — are wired below.

use crate::world::progress::Progress;

/// Whether holding this item summons the Demolitionist.
///
/// `ItemID.Sets.ItemsThatCountAsBombsForDemolitionistToSpawn` (`ItemID.cs:68`), which is exactly
/// what `NPC.SpawnAllowed_Demolitionist` (`NPC.cs:7094-7117`) scans the inventory for.
pub fn counts_as_explosive(id: i32) -> bool {
    const BOMBS: [i32; 21] = [
        166, 167, 168, 235, 1130, 1168, 2586, 2896, 3115, 3116, 3196, 3547, 4423, 4824, 4825, 4826,
        4827, 4908, 4909, 5594, 5595,
    ];
    BOMBS.contains(&id)
}

/// Whether holding this item summons the Arms Dealer.
///
/// `NPC.SpawnAllowed_ArmsDealer` (`NPC.cs:7119-7141`) accepts any item with `ammo == AmmoID.Bullet
/// || useAmmo == AmmoID.Bullet`: bullets, or a gun that fires them. This is every item whose
/// `SetDefaults` (`Item.cs`) sets `ammo`/`useAmmo` to `AmmoID.Bullet`, transcribed by id because
/// the server has no item-ammo table. The old set instead named a Wooden Sword (24), two bows
/// (39, 99), a Molten Fury (120) and the Suspicious Looking Eye boss-summon (43).
pub fn counts_as_gun(id: i32) -> bool {
    const BULLET_ITEMS: [i32; 40] = [
        95, 96, 97, 98, 164, 219, 234, 278, 434, 515, 533, 534, 546, 679, 800, 964, 1179, 1254,
        1255, 1265, 1302, 1319, 1335, 1342, 1349, 1350, 1351, 1352, 1553, 1870, 1929, 2269, 2270,
        2797, 3104, 3475, 3567, 3788, 4915, 5117,
    ];
    BULLET_ITEMS.contains(&id)
}

/// Whether holding this item summons the Dye Trader.
///
/// `NPC.SpawnAllowed_DyeTrader` (`NPC.cs:7144-7183`) accepts any finished dye (`item.dye > 0`,
/// which needs an item-dye table the server does not have and is disclosed as a gap) or a dye
/// plant / Strange Plant reward (`item.type` in 1107..=1120 or 3385..=3388). The old set instead
/// began at 1105, catching Orichalcum (1105) and Titanium (1106) ore.
pub fn counts_as_dye_material(id: i32) -> bool {
    matches!(id, 1107..=1120 | 3385..=3388)
}

/// Copper coins the party must hold between them before the Merchant will come. 50 silver.
const MERCHANT_COINS: i64 = 5_000;

/// The life total that convinces the Nurse there is work here.
///
/// `NPC.SpawnAllowed_Nurse` (`NPC.cs:7185-7199`) is the integer test `statLifeMax / 20 > 5`, not a
/// straight `> 100`: with truncating division that first passes at 120 max life (six life
/// crystals), so 101..=119 does *not* summon her. The old constant `> 100` let her in at 101.
fn nurse_wants_work(best_life: i32) -> bool {
    best_life / 20 > 5
}

/// What the world looks like to somebody deciding whether to move in.
#[derive(Debug, Clone, Copy)]
pub struct Town<'a> {
    pub progress: &'a Progress,
    /// Copper-equivalent coins held across every player.
    pub coins: i64,
    /// The largest maximum life of any player.
    pub best_life: i32,
    /// Whether anybody is carrying an explosive, and whether anybody has a gun.
    pub has_explosives: bool,
    pub has_gun: bool,
    pub has_dye_material: bool,
    /// How many townsfolk already live here.
    pub residents: usize,
    pub hard_mode: bool,
    /// Whether a genuine party is happening right now (`BirthdayParty.GenuineParty`), which is
    /// what a town Green Slime moves in during.
    pub party: bool,
}

/// One resident and the reason they are not here yet.
pub struct Arrival {
    pub npc_type: u16,
    /// What to say when they turn up, past the standard "has moved in".
    pub name: &'static str,
}

/// Everyone who could move in right now, in the order the game checks them.
///
/// Order matters: the game fills the list and then picks, and several conditions depend on who is
/// already resident — the Nurse and Demolitionist both wait for the Merchant, so the Merchant has
/// to be able to arrive first.
pub fn ready(town: Town<'_>, present: &dyn Fn(u16) -> bool) -> Vec<Arrival> {
    let p = town.progress;
    let merchant_here = present(17);

    let candidates: [(u16, &'static str, bool); 14] = [
        (17, "Merchant", town.coins >= MERCHANT_COINS),
        (
            18,
            "Nurse",
            nurse_wants_work(town.best_life) && merchant_here,
        ),
        (19, "Arms Dealer", town.has_gun),
        (
            20,
            "Dryad",
            p.downed_boss1 || p.downed_boss2 || p.downed_boss3,
        ),
        (38, "Demolitionist", town.has_explosives && merchant_here),
        (54, "Clothier", p.downed_boss3),
        (107, "Goblin Tinkerer", p.saved_goblin),
        (108, "Wizard", p.saved_wizard),
        (124, "Mechanic", p.saved_mechanic),
        (178, "Steampunker", p.downed_mech_any),
        (
            207,
            "Dye Trader",
            town.has_dye_material && town.residents >= 4,
        ),
        (228, "Witch Doctor", p.downed_queen_bee),
        (229, "Pirate", p.downed_pirates),
        (227, "Painter", town.residents >= 8),
    ];

    let mut out: Vec<Arrival> = candidates
        .into_iter()
        .filter(|(kind, _, allowed)| *allowed && !present(*kind))
        .map(|(npc_type, name, _)| Arrival { npc_type, name })
        .collect();

    // Hardmode residents, and the ones that wait on a rescue that only happens in hardmode.
    if town.hard_mode {
        for (kind, name, allowed) in [
            (209u16, "Cyborg", p.downed_plantera),
            (353, "Stylist", p.saved_stylist),
            (369, "Angler", p.saved_angler),
            (441, "Tax Collector", p.saved_tax_collector),
            (550, "Tavernkeep", p.saved_bartender),
            (588, "Golfer", p.saved_golfer),
        ] {
            if allowed && !present(kind) {
                out.push(Arrival {
                    npc_type: kind,
                    name,
                });
            }
        }
    } else {
        // The Angler, Stylist, Golfer and Tax Collector are all pre-hardmode rescues too.
        for (kind, name, allowed) in [
            (353u16, "Stylist", p.saved_stylist),
            (369, "Angler", p.saved_angler),
            (441, "Tax Collector", p.saved_tax_collector),
            (588, "Golfer", p.saved_golfer),
            (550, "Tavernkeep", p.saved_bartender),
        ] {
            if allowed && !present(kind) {
                out.push(Arrival {
                    npc_type: kind,
                    name,
                });
            }
        }
    }

    // Residents that had no condition wired at all and so could never move in (`Main.cs`'s own
    // `townNPCCanSpawn` chain, 66755-66940).
    //
    // Truffle: hardmode (`Main.cs:66857`). Vanilla additionally needs a surface-mushroom house,
    // which `CheckSpecialTownNPCSpawningConditions` tests and our housing does not yet classify by
    // biome; that narrowing is disclosed, not papered over.
    if town.hard_mode && !present(160) {
        out.push(Arrival {
            npc_type: 160,
            name: "Truffle",
        });
    }
    // Party Girl: 20 or more townsfolk already living here (`Main.cs:66770-66775,66869`, where
    // `flag7 = rand(40)==0 && num40 >= 20`). The 1-in-40 roll is dropped: the arrival tick is
    // already infrequent, and threading an RNG through this otherwise-pure decision to model a
    // per-check coin flip is not worth it. Disclosed.
    if town.residents >= 20 && !present(208) {
        out.push(Arrival {
            npc_type: 208,
            name: "Party Girl",
        });
    }
    // Green Slime: a town slime that moves in while a genuine party is happening
    // (`Main.cs:66780,66901`, `flag8 = BirthdayParty.GenuineParty`). This is one of the two things
    // the un-wired residents blocked: natural parties never gained their party-time slime.
    if town.party && !present(678) {
        out.push(Arrival {
            npc_type: 678,
            name: "Green Slime",
        });
    }
    // Princess: everyone else is already home (`Main.cs:66929-66940`, `flag9`: every ordinary town
    // resident num2..num27 present). Santa, the pets and the researched slimes are not part of the
    // roster, but the Zoologist is - so in practice the Princess is gated behind the Zoologist,
    // whom the server cannot yet place (see the module note); the condition is transcribed
    // faithfully all the same.
    if PRINCESS_ROSTER.iter().all(|kind| present(*kind)) && !present(663) {
        out.push(Arrival {
            npc_type: 663,
            name: "Princess",
        });
    }

    out
}

/// The ordinary town residents that must all be home before the Princess will come
/// (`Main.cs:66929`'s `flag9`, `num2..num27`): Merchant, Nurse, Arms Dealer, Dryad, Guide,
/// Demolitionist, Clothier, Wizard, Goblin Tinkerer, Mechanic, Truffle, Steampunker, Dye Trader,
/// Party Girl, Cyborg, Painter, Witch Doctor, Pirate, Stylist, Angler, Tax Collector, Tavernkeep,
/// Golfer and Zoologist. Santa, the pets and the town slimes are deliberately not in it.
const PRINCESS_ROSTER: [u16; 24] = [
    17, 18, 19, 20, 22, 38, 54, 107, 108, 124, 160, 178, 207, 208, 209, 227, 228, 229, 353, 369,
    441, 550, 588, 633,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_town(progress: &Progress) -> Town<'_> {
        Town {
            progress,
            coins: 0,
            best_life: 100,
            has_explosives: false,
            has_gun: false,
            has_dye_material: false,
            residents: 1,
            hard_mode: false,
            party: false,
        }
    }

    fn nobody(_: u16) -> bool {
        false
    }

    /// The item triggers name the right items. Before the fix a Wooden Sword (24) summoned the
    /// Arms Dealer and ore (1105/1106) the Dye Trader (`NPC.cs:7119-7183`, `ItemID.cs:68`).
    #[test]
    fn the_item_triggers_name_the_right_items() {
        // Arms Dealer: bullets and bullet-firing guns, not swords, bows or boss summons.
        assert!(counts_as_gun(96), "the Musket is a gun");
        assert!(counts_as_gun(97), "a Musket Ball is bullet ammo");
        assert!(counts_as_gun(98), "the Minishark is a gun");
        assert!(!counts_as_gun(24), "a Wooden Sword is not a gun");
        assert!(!counts_as_gun(39), "a Wooden Bow is not a gun");
        assert!(!counts_as_gun(99), "an Iron Bow is not a gun");
        assert!(!counts_as_gun(43), "a Suspicious Looking Eye is not a gun");

        // Dye Trader: dye plants and Strange Plants, not ore.
        assert!(
            counts_as_dye_material(1107),
            "a Teal Mushroom is dye material"
        );
        assert!(counts_as_dye_material(1120), "a Dye Vat counts too");
        assert!(
            counts_as_dye_material(3385),
            "a Strange Plant is dye material"
        );
        assert!(
            !counts_as_dye_material(1105),
            "Orichalcum ore is not dye material"
        );
        assert!(
            !counts_as_dye_material(1106),
            "Titanium ore is not dye material"
        );

        // Demolitionist: real explosives.
        assert!(counts_as_explosive(166), "a Bomb");
        assert!(counts_as_explosive(167), "Dynamite");
        assert!(
            !counts_as_explosive(1),
            "an Iron Pickaxe is not an explosive"
        );
    }

    /// A fresh world with a poor player has nobody waiting to move in.
    #[test]
    fn an_empty_world_attracts_nobody() {
        let progress = Progress::default();
        assert!(ready(empty_town(&progress), &nobody).is_empty());
    }

    /// Fifty silver brings the Merchant, and nobody else.
    #[test]
    fn coins_bring_the_merchant() {
        let progress = Progress::default();
        let town = Town {
            coins: MERCHANT_COINS,
            ..empty_town(&progress)
        };
        let waiting = ready(town, &nobody);
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].npc_type, 17);
    }

    /// The Nurse waits for the Merchant, however much life the player has.
    #[test]
    fn the_nurse_waits_for_the_merchant() {
        let progress = Progress::default();
        let town = Town {
            best_life: 200,
            ..empty_town(&progress)
        };
        assert!(
            !ready(town, &nobody).iter().any(|a| a.npc_type == 18),
            "the Nurse should not arrive before the Merchant",
        );
        assert!(
            ready(town, &|kind| kind == 17)
                .iter()
                .any(|a| a.npc_type == 18),
            "with the Merchant here she should come",
        );
    }

    /// Beating any of the first three bosses brings the Dryad.
    #[test]
    fn a_boss_brings_the_dryad() {
        let progress = Progress {
            downed_boss2: true,
            ..Default::default()
        };
        let town = empty_town(&progress);
        assert!(ready(town, &nobody).iter().any(|a| a.npc_type == 20));
    }

    /// A rescued Mechanic moves in — which is the only way wire ever reaches a player.
    #[test]
    fn a_rescued_mechanic_moves_in() {
        let progress = Progress {
            saved_mechanic: true,
            ..Default::default()
        };
        let town = empty_town(&progress);
        let waiting = ready(town, &nobody);
        assert!(
            waiting.iter().any(|a| a.npc_type == 124),
            "no Mechanic means no wire, and the whole wiring system is unreachable",
        );
    }

    /// The Nurse waits for 120 max life, not 101: `statLifeMax / 20 > 5` with truncating integer
    /// division (`NPC.cs:7185-7199`). Fails before the fix, when the threshold was a flat `> 100`.
    #[test]
    fn the_nurse_uses_the_real_life_formula() {
        assert!(!nurse_wants_work(100), "five crystals is not enough");
        assert!(!nurse_wants_work(119), "nor is 119, since 119 / 20 == 5");
        assert!(
            nurse_wants_work(120),
            "six crystals (120) is the real threshold"
        );

        let progress = Progress::default();
        let just_short = Town {
            best_life: 110,
            ..empty_town(&progress)
        };
        assert!(
            !ready(just_short, &|kind| kind == 17)
                .iter()
                .any(|a| a.npc_type == 18),
            "110 max life should not summon the Nurse even with the Merchant home",
        );
    }

    /// Four townsfolk that could never move in now can. Before the fix none of them were in the
    /// candidate list at all (`Main.cs:66755-66940`).
    #[test]
    fn the_unwired_townsfolk_can_move_in() {
        use terrustia_proto::npc_data::npc_stats;
        for kind in [160u16, 208, 678, 663] {
            assert!(npc_stats(kind).is_some(), "{kind} is a real NPC type");
        }

        // Truffle waits on hardmode.
        let progress = Progress::default();
        let soft = empty_town(&progress);
        assert!(!ready(soft, &nobody).iter().any(|a| a.npc_type == 160));
        let hard = Town {
            hard_mode: true,
            ..empty_town(&progress)
        };
        assert!(
            ready(hard, &nobody).iter().any(|a| a.npc_type == 160),
            "hardmode brings the Truffle, and with him Shroomite",
        );

        // Party Girl waits on a crowd of 20.
        let small = Town {
            residents: 10,
            ..empty_town(&progress)
        };
        assert!(!ready(small, &nobody).iter().any(|a| a.npc_type == 208));
        let crowd = Town {
            residents: 20,
            ..empty_town(&progress)
        };
        assert!(ready(crowd, &nobody).iter().any(|a| a.npc_type == 208));

        // Green Slime waits on a party.
        let quiet = empty_town(&progress);
        assert!(!ready(quiet, &nobody).iter().any(|a| a.npc_type == 678));
        let partying = Town {
            party: true,
            ..empty_town(&progress)
        };
        assert!(
            ready(partying, &nobody).iter().any(|a| a.npc_type == 678),
            "a genuine party brings a town Green Slime",
        );

        // Princess waits on the whole roster being home.
        let full = empty_town(&progress);
        let roster_present = |kind: u16| super::PRINCESS_ROSTER.contains(&kind);
        assert!(
            ready(full, &roster_present)
                .iter()
                .any(|a| a.npc_type == 663),
            "with every ordinary resident home the Princess arrives",
        );
        assert!(
            !ready(full, &nobody).iter().any(|a| a.npc_type == 663),
            "but not to an empty town",
        );
    }

    /// Somebody already resident is not invited twice.
    #[test]
    fn nobody_arrives_twice() {
        let progress = Progress {
            downed_boss1: true,
            ..Default::default()
        };
        let town = empty_town(&progress);
        assert!(
            ready(town, &|kind| kind == 20)
                .iter()
                .all(|a| a.npc_type != 20)
        );
    }
}
