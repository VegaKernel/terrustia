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

use crate::world::progress::Progress;

/// Copper coins the party must hold between them before the Merchant will come. 50 silver.
const MERCHANT_COINS: i64 = 5_000;

/// The life total that convinces the Nurse there is work here: `statLifeMax / 20 > 5`.
const NURSE_LIFE: i32 = 100;

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
        (18, "Nurse", town.best_life > NURSE_LIFE && merchant_here),
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

    out
}

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
        }
    }

    fn nobody(_: u16) -> bool {
        false
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
