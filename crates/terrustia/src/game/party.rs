//! The birthday party — a self-scheduling one-day celebration among town NPCs.
//!
//! `Terraria.GameContent.Events.BirthdayParty`. Real vanilla rolls for one every morning
//! (`Main.UpdateTime_StartDay` calling `CheckMorning`) and always ends it at the next nightfall
//! (`UpdateTime_StartNight` calling `CheckNight`) — genuine *and* manually-forced parties alike,
//! which is why there is no persistent "party mode" setting, only a state that resets nightly.
//!
//! The real gameplay footprint turns out to be almost entirely cosmetic once traced: party hats on
//! every town NPC (`NPC.GetPartyHatOrShimmerHead`, `NPC.cs:91324`), a sky effect, a handful of
//! dialogue lines (`freeCake`, a one-shot condition on a single line of Guide chatter,
//! `ConditionalDialogue.cs`), and the announcement naming who is celebrating. No shop-price effect,
//! no spawn-table effect, no combat effect — confirmed by grepping every consumer of
//! `BirthdayParty.PartyIsUp` in source, not assumed from the category. The one *server-relevant*
//! piece worth a real mechanism is what makes a party start or end at all, which is what this
//! module models.

use rand::{Rng, rngs::SmallRng, seq::SliceRandom};

/// `NPCID.Sets.IsTownPet` (`NPCID.cs:4446`) — a town pet is otherwise indistinguishable from an
/// ordinary town NPC (`town_npc: true`, an ordinary `ai_style`), so this is the only way to keep
/// pets out of who a party can be thrown for or with.
const TOWN_PETS: [u16; 11] = [637, 638, 656, 670, 678, 679, 680, 681, 682, 683, 684];

/// `BirthdayParty::CanNPCParty`'s own three named exceptions. Each is a liminal or transformed
/// identity rather than an ordinary settled resident: the Old Man becomes Skeletron at night, and
/// the Tax Collector and Skeleton Merchant are both post-transformation identities other town NPCs
/// take on, not residents of their own.
const CANNOT_PARTY: [u16; 3] = [37, 441, 453];

/// Party Girl, `npc_type` 208 — real vanilla's own gate (`NPC.AnyNPCs(208)`): no natural party can
/// ever start in a world she has not moved into.
pub const PARTY_GIRL: u16 = 208;

/// State for one world's birthday party. In-memory only — real vanilla never saves any of
/// `BirthdayParty`'s fields to the `.wld` file either (unlike `Main.slimeRainTime`, which the file
/// format does carry — see `slime_rain.rs`'s own module doc), so there is no persistence gap to
/// disclose here at all: a party or a cooldown in progress when the server stops is meant to be
/// gone, the same as it would be after real vanilla's own singleplayer session ends.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PartyState {
    pub genuine: bool,
    pub manual: bool,
    /// Nights left before a natural party can be rolled for again.
    pub days_on_cooldown: i32,
    /// NPC indices celebrating this party — one, two, or three of them, real vanilla's own
    /// `CelebratingNPCs`. Empty for a manual-only party: nobody is named in that announcement.
    pub celebrating: Vec<u8>,
}

impl PartyState {
    /// `BirthdayParty.PartyIsUp`.
    pub fn is_up(&self) -> bool {
        self.genuine || self.manual
    }

    /// `BirthdayParty::CanNPCParty` — eligible to be one of the celebrating NPCs, or to count
    /// toward the five needed for a natural party to start at all.
    pub fn can_party(npc_type: u16, town_npc: bool, ai_style: i32) -> bool {
        town_npc
            && ai_style != 0
            && !CANNOT_PARTY.contains(&npc_type)
            && !TOWN_PETS.contains(&npc_type)
    }

    /// `BirthdayParty::NaturalAttempt`, called once at dawn. `party_girl_present` is `NPC.AnyNPCs
    /// (208)`; `eligible` is every present town NPC's index that [`can_party`] allows. Real
    /// vanilla's own two-roll count selection (`rand.Next(5)==0 && count>12`, else `rand.Next(3)
    /// ==0`, else one) is transcribed with its exact short-circuit shape preserved: the second roll
    /// only happens when the first one either failed outright or won without a big enough pool,
    /// never both rolls unconditionally. Returns the indices chosen to celebrate, if a party
    /// started — `self.celebrating` already holds the same value, this is only for the caller's
    /// own announcement.
    pub fn natural_attempt(
        &mut self,
        party_girl_present: bool,
        eligible: &[u8],
        rng: &mut SmallRng,
    ) -> Option<Vec<u8>> {
        if !party_girl_present {
            return None;
        }
        if self.days_on_cooldown > 0 {
            self.days_on_cooldown -= 1;
            return None;
        }
        if rng.random_range(0..10) != 0 {
            return None;
        }
        if eligible.len() < 5 {
            return None;
        }
        self.genuine = true;
        self.days_on_cooldown = rng.random_range(5..11);
        let mut shuffled: Vec<u8> = eligible.to_vec();
        shuffled.shuffle(rng);
        let count = if rng.random_range(0..5) == 0 && eligible.len() > 12 {
            3
        } else if rng.random_range(0..3) == 0 {
            2
        } else {
            1
        };
        self.celebrating = shuffled.into_iter().take(count).collect();
        Some(self.celebrating.clone())
    }

    /// `BirthdayParty::UpdateTime`'s per-tick prune: an NPC that stops being eligible mid-day
    /// (killed, evicted, whatever) is dropped from the celebration, and a genuine party with nobody
    /// left to celebrate ends early. `still_eligible` should reflect [`can_party`] against the
    /// NPC's *current* state (it may since have been evicted, though its type cannot change).
    /// Returns whether a genuine party just ended this way, so the caller knows to announce it —
    /// real vanilla does not re-announce when a *manual* party's own celebrants (it has none) run
    /// out, only a genuine one's.
    pub fn prune(&mut self, still_eligible: impl Fn(u8) -> bool) -> bool {
        if !self.genuine || self.celebrating.is_empty() {
            return false;
        }
        self.celebrating.retain(|&index| still_eligible(index));
        if self.celebrating.is_empty() {
            self.genuine = false;
            return true;
        }
        false
    }

    /// `BirthdayParty::CheckNight` — a party never survives past one day, genuine or manually
    /// forced alike. Returns whether anything was actually up to end, so the caller knows whether
    /// to announce.
    pub fn end_for_the_night(&mut self) -> bool {
        let was_up = self.is_up();
        self.genuine = false;
        self.manual = false;
        self.celebrating.clear();
        was_up
    }

    /// `BirthdayParty::ToggleManualParty` — the Party Monolith's own effect
    /// (`wiring.rs`'s `PARTY_MONOLITH`). Returns the new `is_up()` state, so the caller knows
    /// whether to announce a start or an end.
    pub fn toggle_manual(&mut self) -> bool {
        self.manual = !self.manual;
        self.is_up()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(7)
    }

    #[test]
    fn no_party_girl_means_no_natural_party() {
        let mut state = PartyState::default();
        let eligible: Vec<u8> = (0..10).collect();
        assert_eq!(state.natural_attempt(false, &eligible, &mut rng()), None);
        assert!(!state.genuine);
    }

    #[test]
    fn fewer_than_five_eligible_means_no_natural_party() {
        let mut state = PartyState::default();
        let eligible: Vec<u8> = (0..4).collect();
        // Loop past the daily-roll chance so a fresh cooldown does not mask this on its own.
        for _ in 0..200 {
            state.days_on_cooldown = 0;
            assert_eq!(state.natural_attempt(true, &eligible, &mut rng()), None);
        }
        assert!(!state.genuine);
    }

    #[test]
    fn a_cooldown_counts_down_rather_than_letting_a_party_start() {
        let mut state = PartyState {
            days_on_cooldown: 3,
            ..Default::default()
        };
        let eligible: Vec<u8> = (0..10).collect();
        for expected in [2, 1, 0] {
            assert_eq!(state.natural_attempt(true, &eligible, &mut rng()), None);
            assert_eq!(state.days_on_cooldown, expected);
        }
    }

    /// With the daily roll guaranteed to keep trying, a pool of exactly five eventually throws a
    /// party — and never picks more than are actually present.
    #[test]
    fn a_natural_party_eventually_starts_and_picks_from_who_is_there() {
        let eligible: Vec<u8> = (0..5).collect();
        let mut r = rng();
        for _ in 0..2000 {
            let mut state = PartyState::default();
            if let Some(chosen) = state.natural_attempt(true, &eligible, &mut r) {
                assert!(state.genuine);
                assert!(!chosen.is_empty() && chosen.len() <= 3);
                assert!(chosen.iter().all(|c| eligible.contains(c)));
                assert!((5..11).contains(&state.days_on_cooldown));
                return;
            }
        }
        panic!("a party should have started at least once in 2000 tries");
    }

    #[test]
    fn old_man_tax_collector_skeleton_merchant_and_pets_cannot_party() {
        for &excluded in CANNOT_PARTY.iter().chain(TOWN_PETS.iter()) {
            assert!(
                !PartyState::can_party(excluded, true, 7),
                "{excluded} should never be eligible"
            );
        }
        assert!(
            PartyState::can_party(PARTY_GIRL, true, 7),
            "an ordinary town NPC should be eligible"
        );
    }

    #[test]
    fn a_non_town_npc_cannot_party_regardless_of_type() {
        assert!(
            !PartyState::can_party(1, false, 7),
            "a slime is not a town NPC"
        );
    }

    #[test]
    fn zero_ai_style_cannot_party() {
        assert!(!PartyState::can_party(PARTY_GIRL, true, 0));
    }

    #[test]
    fn night_ends_both_a_genuine_and_a_manual_party() {
        let mut genuine = PartyState {
            genuine: true,
            celebrating: vec![3, 7],
            ..Default::default()
        };
        assert!(genuine.end_for_the_night());
        assert!(!genuine.is_up());
        assert!(genuine.celebrating.is_empty());

        let mut manual = PartyState {
            manual: true,
            ..Default::default()
        };
        assert!(manual.end_for_the_night());
        assert!(!manual.is_up());

        let mut nothing = PartyState::default();
        assert!(!nothing.end_for_the_night(), "nothing was up to end");
    }

    #[test]
    fn toggling_manual_flips_is_up_independent_of_genuine() {
        let mut state = PartyState::default();
        assert!(state.toggle_manual());
        assert!(state.is_up());
        assert!(!state.toggle_manual());
        assert!(!state.is_up());
    }

    #[test]
    fn a_genuine_party_survives_losing_one_celebrant_but_not_all() {
        let mut state = PartyState {
            genuine: true,
            celebrating: vec![1, 2, 3],
            ..Default::default()
        };
        assert!(!state.prune(|i| i != 2), "still two left, party continues");
        assert_eq!(state.celebrating, vec![1, 3]);
        assert!(state.genuine);

        assert!(state.prune(|_| false), "nobody left, party ends early");
        assert!(!state.genuine);
        assert!(state.celebrating.is_empty());
    }

    #[test]
    fn pruning_never_touches_a_manual_only_party() {
        let mut state = PartyState {
            manual: true,
            ..Default::default()
        };
        assert!(!state.prune(|_| false));
        assert!(state.manual, "a manual party has no celebrants to lose");
    }
}
