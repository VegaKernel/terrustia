//! Lantern Night — `Terraria.GameContent.Events.LanternNight`, the third of the three missing
//! events.
//!
//! Real vanilla's own footprint is almost entirely cosmetic once traced, the same shape Birthday
//! Party's own module doc already found for its event: a sky effect (`SkyManager`'s "Lantern"
//! layer), a Guide dialogue line, and — the one piece worth a real look — the Traveling Merchant
//! gaining the Release Lantern (`ItemID.ReleaseLantern`, 4702) to his stock while it's up
//! (`Chest.cs`'s own shop-building code, a plain `if (LanternNight.LanternsUp)` appended after the
//! chain-of-rolls that builds the rest of his inventory — not integrated into that chain at all).
//! `terrustia_proto::travel_shop`'s own `OFFERS` table is generated from that exact chain by
//! `tools/gen_travel_shop.py` and gated only on permanent world progress (`Needs`, a bitset) —
//! Lantern Night is a transient *current event*, not progress, and does not fit that shape without
//! touching the generator; disclosed-skipped rather than hand-edited into a table the file's own
//! header says not to touch by hand.
//!
//! **The one real, server-relevant mechanism is what starts or ends the night itself**, which is
//! what this module models. `CheckNight` (dusk) rolls for a natural one; `CheckMorning` (dawn)
//! clears whatever was up, genuine or manual alike — the opposite timing from Birthday Party
//! (a day event, rolled at dawn, cleared at dusk), matching each event's own real time of day.
//!
//! **The natural roll has no population requirement at all** — no Party-Girl-style NPC gate —
//! just a cooldown and a flat 1-in-14 daily chance, gated entirely on Moon Lord ever having been
//! downed (`NPC.downedMoonlord`). Real vanilla's own `LanternsCanStart` additionally refuses while
//! a blood moon, either moon event, a real invasion, a meteor is owed, or a boss (including the
//! Eater of Worlds' three segments specifically, `NPCID` 13-15, none of which carry the ordinary
//! `.boss` flag) is already up — computing that gate is the caller's job (`server.rs`'s own
//! `roll_natural_lantern_night`), not this module's, since every one of those inputs lives
//! elsewhere already.
//!
//! **`NextNightIsLanternNight` — a real mechanism separate from the daily roll, not modelled as an
//! afterthought**: real vanilla guarantees the *next* roll succeeds outright the first time any of
//! 22 real `gameEventId`s (`NPC.OnGameEventClearedForTheFirstTime`) is ever cleared — nearly every
//! boss kill, plus the wall falling to start hardmode. Counted directly against source rather than
//! estimated: 5 of the 22 are pumpkin/snow-moon-gated Halloween/Christmas event bosses this
//! project does not have at all; the remaining 17 are exactly the 17 `downed_*`/`hard_mode`
//! transitions `server.rs`'s own pre-existing `note_boss_kill` dispatcher already makes, one for
//! one. The Old One's Army's own boss does not set this flag either, confirmed by direct
//! inspection of `DD2Event.cs` finding no mention of `LanternNight` anywhere in it at all — so the
//! guarantee is wired as one snapshot-and-diff check around the whole existing dispatcher rather
//! than 17 individual call sites, catching every real transition it already makes without
//! duplicating its own boss roster by hand.
//!
//! **What this does not model**: `ToggleManualLanterns` (real vanilla's own manual-forcing
//! method) exists in source but — confirmed directly, `grep -rl ToggleManualLanterns` across the
//! entire decompiled tree finds no caller anywhere — has no real player-facing trigger at all,
//! unlike Birthday Party's genuine Party Monolith tile. The `manual` field and [`toggle_manual`]
//! method below exist for fidelity to the class's own shape (and so `is_up()` matches
//! `LanternsUp`'s real `genuine ? true : manual` logic exactly), but nothing in this project wires
//! anything to call it, honestly reflecting that real vanilla itself has nothing to either.

use rand::{Rng, rngs::SmallRng};

/// State for one world's lantern night. In-memory only, the same disclosed shape `party.rs`'s own
/// module doc already established: real vanilla never saves any of `LanternNight`'s own fields to
/// the `.wld` file either, so there is no persistence gap here to disclose.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct LanternNightState {
    pub genuine: bool,
    pub manual: bool,
    /// Nights left before a natural lantern night can be rolled for again.
    pub days_on_cooldown: i32,
    /// `LanternNight.NextNightIsLanternNight` — set once a real vanilla "game event" clears for
    /// the first time; guarantees the very next roll succeeds outright regardless of the cooldown
    /// or the daily chance.
    pub next_night_guaranteed: bool,
}

impl LanternNightState {
    /// `LanternNight.LanternsUp`.
    pub fn is_up(&self) -> bool {
        self.genuine || self.manual
    }

    /// `LanternNight::NaturalAttempt`, called once at dusk. `can_start` is real vanilla's own
    /// `LanternsCanStart()` — the caller's responsibility, since every one of its inputs (blood
    /// moon, the moon events, an invasion, an owed meteor, any boss up including the Eater of
    /// Worlds' three segments) lives on the caller's own state, not here. `downed_moon_lord` is
    /// `NPC.downedMoonlord` — the one real gate on the roll firing at all.
    pub fn natural_attempt(&mut self, can_start: bool, downed_moon_lord: bool, rng: &mut SmallRng) {
        if !can_start {
            return;
        }
        // The guarantee fires outright, bypassing the cooldown and the daily roll entirely — real
        // vanilla's own "the next roll succeeds regardless of cooldown or odds" shape, checked
        // first so an armed guarantee can never be silently absorbed by an ordinary cooldown tick.
        if self.next_night_guaranteed {
            self.next_night_guaranteed = false;
            self.genuine = true;
            self.days_on_cooldown = rng.random_range(5..11);
            return;
        }
        // A cooldown tick and the daily roll are mutually exclusive within one call — the same
        // shape `party.rs`'s own already-shipped `natural_attempt` established for its own
        // analogous cooldown, so the exact night the cooldown reaches zero never *also* rolls in
        // that same call.
        if self.days_on_cooldown > 0 {
            self.days_on_cooldown -= 1;
            return;
        }
        if downed_moon_lord && rng.random_range(0..14) == 0 {
            self.genuine = true;
            self.days_on_cooldown = rng.random_range(5..11);
        }
    }

    /// `LanternNight::CheckMorning` — a lantern night never survives past one dawn, genuine or
    /// manually forced alike. Returns whether anything was actually up to end, so the caller knows
    /// whether to announce.
    pub fn end_for_the_morning(&mut self) -> bool {
        let was_up = self.is_up();
        self.genuine = false;
        self.manual = false;
        was_up
    }

    /// `LanternNight::ToggleManualLanterns` — kept for fidelity to the real class's own shape; see
    /// this module's own doc comment on why nothing in this project actually calls it. Returns the
    /// new `is_up()` state.
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
    fn no_moon_lord_kill_means_no_natural_lantern_night() {
        let mut state = LanternNightState::default();
        for _ in 0..500 {
            state.days_on_cooldown = 0;
            state.natural_attempt(true, false, &mut rng());
        }
        assert!(!state.genuine);
    }

    #[test]
    fn the_gate_being_false_refuses_a_roll_even_with_everything_else_ready() {
        let mut state = LanternNightState::default();
        for _ in 0..500 {
            state.days_on_cooldown = 0;
            state.natural_attempt(false, true, &mut rng());
        }
        assert!(!state.genuine, "can_start=false should refuse outright");
    }

    #[test]
    fn a_cooldown_counts_down_rather_than_letting_a_night_start() {
        let mut state = LanternNightState {
            days_on_cooldown: 3,
            ..Default::default()
        };
        for expected in [2, 1, 0] {
            state.natural_attempt(true, true, &mut rng());
            assert_eq!(state.days_on_cooldown, expected);
        }
    }

    /// With the daily roll guaranteed to keep trying, a lantern night eventually starts.
    #[test]
    fn a_natural_lantern_night_eventually_starts() {
        let mut r = rng();
        for _ in 0..2000 {
            let mut state = LanternNightState::default();
            state.natural_attempt(true, true, &mut r);
            if state.genuine {
                assert!((5..11).contains(&state.days_on_cooldown));
                return;
            }
        }
        panic!("a lantern night should have started at least once in 2000 tries");
    }

    /// The guarantee fires on its own, ignoring the cooldown and the daily roll entirely.
    #[test]
    fn the_guaranteed_flag_starts_a_night_regardless_of_cooldown_or_odds() {
        let mut state = LanternNightState {
            days_on_cooldown: 9,
            next_night_guaranteed: true,
            ..Default::default()
        };
        // Downed Moon Lord is irrelevant to the guarantee itself — only to the ordinary roll.
        state.natural_attempt(true, false, &mut rng());
        assert!(state.genuine, "the guarantee should have fired outright");
        assert!(
            !state.next_night_guaranteed,
            "consumed once it fires, not left armed"
        );
    }

    #[test]
    fn morning_ends_both_a_genuine_and_a_manual_night() {
        let mut genuine = LanternNightState {
            genuine: true,
            ..Default::default()
        };
        assert!(genuine.end_for_the_morning());
        assert!(!genuine.is_up());

        let mut manual = LanternNightState {
            manual: true,
            ..Default::default()
        };
        assert!(manual.end_for_the_morning());
        assert!(!manual.is_up());

        let mut nothing = LanternNightState::default();
        assert!(!nothing.end_for_the_morning(), "nothing was up to end");
    }

    #[test]
    fn toggling_manual_flips_is_up_independent_of_genuine() {
        let mut state = LanternNightState::default();
        assert!(state.toggle_manual());
        assert!(state.is_up());
        assert!(!state.toggle_manual());
        assert!(!state.is_up());
    }
}
