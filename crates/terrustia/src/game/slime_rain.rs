//! Slime Rain — `Main.slimeRain` and friends (`Terraria/Main.cs`, `Terraria/NPC.cs`).
//!
//! One field, `Main.slimeRainTime`, carries two opposite meanings by its sign: positive while it
//! rains (ticks left), negative as the cooldown afterward (ticks until eligible again, counting
//! *up* toward zero). `NPC.BusyWithAnyInvasionOfSorts`'s own gate on the daily roll reads it as a
//! single `== 0.0` check — busy in either direction — which is why this keeps the same one-field
//! shape rather than splitting it into an `active`/`cooldown` pair: splitting it would have to
//! reconstruct that single check from two fields at every call site instead of asking one.
//!
//! The daily roll itself (`roll`, below) is the real mechanism, transcribed whole from
//! `Main.UpdateTime` rather than assumed to be "start it sometimes": it only runs before noon,
//! only while nothing else the world considers a "special event" is already happening (a shared
//! busy-check with blood moon, eclipse, the two moon events, an invasion, and the Old One's Army —
//! `NPC.BusyWithAnyInvasionOfSorts`'s own list), and its odds shift on three real, independent
//! axes: whether King Slime has ever been beaten, hardmode, and whether anyone present is actually
//! equipped to fight him (`AnyPlayerReadyToFightKingSlime`'s own `statLifeMax` above 140 and
//! `statDefense` above 8) — nobody ready multiplies the already-long odds by five, and without
//! either that or expert mode the roll never happens at all.
//!
//! Killing the right thing during it (`note_kill`, `DoDeathEvents_AdvanceSlimeRain`) escalates
//! toward King Slime arriving on his own, at half the kill count once he has already been beaten
//! once — real vanilla's own way of saying the rain gets less of a novelty the second time. Real
//! vanilla spawns him at the *closest* player to the killing blow (`SpawnOnPlayer(closestPlayer.
//! whoAmI, 50)`), not a random one — the caller needs the death's own position to find that player,
//! which is why this module only decides *whether* to summon him, not where.
//!
//! **The start/stop announcement is delayed, not instant** — a real mechanism found by reading
//! `Main.cs` directly rather than assumed from `StartSlimeRain`/`StopSlimeRain`'s own names alone.
//! Both only arm a 420-tick (`slimeWarningDelay`, about seven real-time seconds) countdown
//! (`slimeWarningTime`); `UpdateSlimeRainWarning` fires the actual message once that reaches zero,
//! reading whichever way `slimeRainTime`'s sign points *at that later moment* — so a stop landing
//! while an earlier start's own warning is still counting down silently swallows the start message
//! rather than showing both, and this module's own `tick_warning` preserves exactly that behaviour
//! by re-arming the same countdown on every `start`/`stop` rather than tracking two independent
//! timers. The real English text (`LegacyWorldGen.74`/`.75`, confirmed against the game's own
//! localization content rather than guessed): "Slime is falling from the sky!" / "Slime has
//! stopped falling from the sky."
//!
//! **What this does not model**: real vanilla's `slimeRainNPC[]` flags exactly one type (Blue
//! Slime, `Main.cs:9003`) for a spawn-slot *discount* during the rain (`0.65×` its ordinary
//! weight against the per-player cap) — this project's own spawn cap is already one shared count
//! across every player rather than vanilla's fully independent per-player weighted one (`spawn.rs`
//! `try_spawn`'s own comment on the same simplification, already made before this event existed),
//! so there is no weighted number here to discount at all; not modelled, for the same reason.
//! `SlimeRainSpawns` also has a small chance (1-in-200, `NPC.cs:5945`) of a rare, specially
//! recoloured and rebalanced slime variant (and further expert/normal-mode odds between two other
//! variants) rather than a plain Blue Slime — those are `SetDefaults_ForNetId` overrides on the
//! same base type, not different creatures, and this project spawns an ordinary Blue Slime in
//! every case instead: a real, disclosed simplification, not a different creature this project is
//! missing outright. The daily roll's own denominator has one more real narrowing division
//! (`num3 /= 5`) on `WorldGen.Skyblock.lowTiles` — a secret-seed-specific world shape this project
//! has no concept of at all (`plan.md`'s own "Secret seeds… deprioritized" line), so [`denominator`]
//! below never takes it, the same class of omission as the rare-variant one above.

use rand::{Rng, rngs::SmallRng};

/// The one Blue-Slime-only spawn-slot discount vanilla applies during a rain (`Main.cs:1132`,
/// `slimeRainNPCSlots`) — kept as a named constant even though nothing here reads it, so the
/// module doc's own disclosure has a real number to point at rather than a vague "some discount".
#[allow(dead_code)]
const BLUE_SLIME_SPAWN_SLOT_WEIGHT: f32 = 0.65;

/// King Slime, `npc_type` 50 — both the escalation's own target and the type
/// `SlimeRainSpawns` never spawns (`!AnyNPCs(50)` guards the kill-count entirely while he is
/// already out).
pub const KING_SLIME: u16 = 50;

/// Blue Slime, `npc_type` 1 — the one type `slimeRainNPC[]` flags, and (barring the rare variant
/// rolls this project does not model) the one `SlimeRainSpawns` actually spawns.
pub const BLUE_SLIME: u16 = 1;

/// `Main.slimeWarningDelay` — how long a start or stop waits before it's actually announced.
const WARNING_DELAY: i32 = 420;

/// State for one world's slime rain.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SlimeRainState {
    /// `Main.slimeRainTime`'s own dual sign: positive while it rains, negative as a cooldown
    /// counting up toward zero, `0` meaning neither.
    pub timer: i32,
    /// `Main.slimeRainKillCount` — reset to `0` when a rain starts, and to `-threshold / 2` (a
    /// negative head start on the *next* King Slime, not a full reset) each time he arrives.
    pub kill_count: i32,
    /// `Main.slimeWarningTime`. Real vanilla does not announce a start or stop the instant it
    /// happens — `StartSlimeRain`/`StopSlimeRain` only arm this countdown (`slimeWarningDelay`,
    /// 420 ticks, about seven real-time seconds at the ordinary clock rate), and
    /// `UpdateSlimeRainWarning` fires the actual message once it reaches zero, reading whichever
    /// way `slimeRainTime`'s sign points *at that later moment* — which is also why a stop that
    /// lands while a start's own warning is still counting down silently swallows the start
    /// message rather than showing both.
    pub warning: i32,
}

impl SlimeRainState {
    /// `Main.slimeRain` itself.
    pub fn is_active(&self) -> bool {
        self.timer > 0
    }

    /// `NPC.BusyWithAnyInvasionOfSorts`'s own `slimeRainTime == 0.0` half of its check — busy
    /// either while it rains or during the cooldown after, never neither.
    fn busy(&self) -> bool {
        self.timer != 0
    }

    /// One tick's countdown, run every tick regardless of day or night (`Main.UpdateTime`'s own
    /// unconditional block, `Main.cs:65828-65843`) — `rate` is the day/night clock's own tick
    /// rate (Journey's `ModifyTimeRate`), the same scaling everything else already gets.
    pub fn tick(&mut self, rate: i32, rng: &mut SmallRng) {
        if self.timer > 0 {
            self.timer -= rate;
            if self.timer <= 0 {
                self.stop(rng);
            }
        } else if self.timer < 0 {
            self.timer = (self.timer + rate).min(0);
        }
    }

    /// `Main.slimeWarningTime`'s own countdown (`UpdateSlimeRainWarning`) — decremented by exactly
    /// one every call regardless of the clock rate, since real vanilla calls it once per real
    /// server tick rather than scaling it the way `tick`'s own day-time-keyed `timer` is. Returns
    /// whether it *just* reached zero and, if so, whether the rain is active right now — the
    /// caller announces the matching message. A stop landing while an earlier start's own warning
    /// is still counting down re-arms it at [`WARNING_DELAY`] (`start`/`stop` both do), so only
    /// the most recent transition's message ever actually fires, matching source exactly.
    pub fn tick_warning(&mut self) -> Option<bool> {
        if self.warning <= 0 {
            return None;
        }
        self.warning -= 1;
        if self.warning <= 0 {
            Some(self.is_active())
        } else {
            None
        }
    }

    /// `Main.StopSlimeRain`'s dedicated-server branch: a fresh cooldown, `-rand.Next(3024, 6048) *
    /// 100` — between roughly five and a bit over eleven real-time hours at the ordinary clock
    /// rate, in the same tick units everything else here uses.
    fn stop(&mut self, rng: &mut SmallRng) {
        self.timer = -(rng.random_range(3024..6048) * 100);
        self.warning = WARNING_DELAY;
    }

    /// `Main.StartSlimeRain`'s dedicated-server branch, called only once [`roll`] below has
    /// already decided to.
    fn start(&mut self, rng: &mut SmallRng) {
        if self.timer <= 0 {
            self.timer = rng.random_range(32400..54000);
        }
        self.warning = WARNING_DELAY;
        self.kill_count = 0;
    }

    /// `Main.UpdateTime`'s own daily roll (`Main.cs:65906-65929`), checked every tick before noon.
    ///
    /// `other_events_busy` is the rest of `NPC.BusyWithAnyInvasionOfSorts`'s own list — a blood
    /// moon, an eclipse, either moon event, an invasion, or the Old One's Army — computed by the
    /// caller, since none of those live in this module. `someone_ready_for_king_slime` is
    /// `AnyPlayerReadyToFightKingSlime`'s own `statLifeMax > 140 && statDefense > 8` gate.
    #[allow(clippy::too_many_arguments)]
    pub fn roll(
        &mut self,
        raining: bool,
        day_time: bool,
        before_noon: bool,
        rate: i32,
        other_events_busy: bool,
        downed_king_slime: bool,
        hard_mode: bool,
        someone_ready_for_king_slime: bool,
        expert: bool,
        rng: &mut SmallRng,
    ) {
        if raining || self.busy() || other_events_busy || !day_time || !before_noon || rate <= 0 {
            return;
        }
        let denominator = Self::denominator(
            rate,
            downed_king_slime,
            hard_mode,
            someone_ready_for_king_slime,
        );
        if denominator > 0
            && (someone_ready_for_king_slime || expert)
            && rng.random_range(0..denominator) == 0
        {
            self.start(rng);
        }
    }

    /// `Main.cs:65908-65924`'s own odds, worked out as one number rather than three scattered
    /// multiplications — a separate function so the arithmetic itself can be pinned exactly
    /// without needing to actually win a roll this long to prove it (the smallest of these, even
    /// at Journey's fastest clock, is still five figures — a real design choice on vanilla's part:
    /// this event is meant to be rare, played out over hours rather than a handful of ticks).
    fn denominator(
        rate: i32,
        downed_king_slime: bool,
        hard_mode: bool,
        someone_ready: bool,
    ) -> i32 {
        let mut denominator = 450_000 / rate;
        if !downed_king_slime {
            denominator /= 2;
        } else if hard_mode {
            denominator = ((denominator as f32) * 1.5) as i32;
        }
        if !someone_ready {
            denominator *= 5;
        }
        denominator
    }

    /// `DoDeathEvents_AdvanceSlimeRain`. Returns whether King Slime should now arrive — the caller
    /// owns actually spawning him, this only decides when to.
    pub fn note_kill(
        &mut self,
        npc_type: u16,
        king_slime_present: bool,
        downed_king_slime: bool,
    ) -> bool {
        if !self.is_active() || npc_type != BLUE_SLIME || king_slime_present {
            return false;
        }
        let threshold = if downed_king_slime { 75 } else { 150 };
        self.kill_count += 1;
        if self.kill_count >= threshold {
            self.kill_count = -threshold / 2;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(11)
    }

    #[test]
    fn a_fresh_state_is_neither_active_nor_busy() {
        let state = SlimeRainState::default();
        assert!(!state.is_active());
        assert!(!state.busy());
    }

    #[test]
    fn the_timer_counts_down_while_active_and_stops_at_zero() {
        let mut state = SlimeRainState {
            timer: 5,
            kill_count: 3,
            ..Default::default()
        };
        state.tick(2, &mut rng());
        assert_eq!(state.timer, 3);
        assert!(state.is_active());

        state.tick(2, &mut rng());
        // 3 - 2 = 1, still positive.
        assert_eq!(state.timer, 1);

        state.tick(2, &mut rng());
        // 1 - 2 = -1 <= 0, so it stops and rolls a fresh negative cooldown instead.
        assert!(
            state.timer < 0,
            "should have rolled a cooldown, got {}",
            state.timer
        );
        assert!(!state.is_active());
        assert!(state.busy(), "a cooldown is still busy");
    }

    #[test]
    fn the_cooldown_counts_up_toward_zero_and_then_stops() {
        let mut state = SlimeRainState {
            timer: -5,
            kill_count: 0,
            ..Default::default()
        };
        state.tick(2, &mut rng());
        assert_eq!(state.timer, -3);
        state.tick(2, &mut rng());
        assert_eq!(state.timer, -1);
        state.tick(2, &mut rng());
        assert_eq!(state.timer, 0, "clamped rather than overshooting past zero");
        assert!(!state.busy());

        // Once at zero, further ticks do nothing to it.
        state.tick(2, &mut rng());
        assert_eq!(state.timer, 0);
    }

    #[test]
    fn a_start_arms_the_warning_and_it_fires_active_once_the_delay_elapses() {
        let mut state = SlimeRainState::default();
        state.start(&mut rng());
        assert_eq!(state.warning, WARNING_DELAY);
        for _ in 0..WARNING_DELAY - 1 {
            assert_eq!(state.tick_warning(), None, "not yet — still counting down");
        }
        assert_eq!(
            state.tick_warning(),
            Some(true),
            "the delay just elapsed while the rain is active"
        );
        // The countdown does not idle at zero forever re-firing — it is armed only by a fresh
        // start/stop, matching `Main.slimeWarningTime`'s own `if (slimeWarningTime <= 0) return;`
        // guard.
        assert_eq!(state.tick_warning(), None);
    }

    #[test]
    fn a_stop_landing_mid_warning_swallows_the_start_message_and_reports_inactive() {
        // Real vanilla re-arms the same single countdown on every start/stop rather than tracking
        // them independently — a stop a moment after a start (well inside the 420-tick delay)
        // means only the *stop*'s own message ever fires, never both.
        let mut state = SlimeRainState {
            timer: 5,
            ..Default::default()
        };
        state.start(&mut rng());
        assert_eq!(state.warning, WARNING_DELAY);
        for _ in 0..10 {
            state.tick_warning();
        }
        assert!(state.warning < WARNING_DELAY, "part-way through the delay");
        state.stop(&mut rng());
        assert_eq!(
            state.warning, WARNING_DELAY,
            "stop re-arms the same countdown from the top"
        );
        for _ in 0..WARNING_DELAY - 1 {
            state.tick_warning();
        }
        assert_eq!(
            state.tick_warning(),
            Some(false),
            "only the stop's own message should ever fire"
        );
    }

    #[test]
    fn the_roll_never_fires_while_raining() {
        let mut state = SlimeRainState::default();
        for _ in 0..500 {
            state.roll(
                true,
                true,
                true,
                1,
                false,
                true,
                false,
                true,
                true,
                &mut rng(),
            );
        }
        assert!(!state.is_active());
    }

    #[test]
    fn the_roll_never_fires_while_busy_with_something_else() {
        let mut state = SlimeRainState::default();
        for _ in 0..500 {
            state.roll(
                false,
                true,
                true,
                1,
                true,
                true,
                false,
                true,
                true,
                &mut rng(),
            );
        }
        assert!(!state.is_active());
    }

    #[test]
    fn the_roll_never_fires_outside_the_daytime_before_noon_window() {
        let mut state = SlimeRainState::default();
        for _ in 0..500 {
            state.roll(
                false,
                false,
                true,
                1,
                false,
                true,
                false,
                true,
                true,
                &mut rng(),
            );
            state.roll(
                false,
                true,
                false,
                1,
                false,
                true,
                false,
                true,
                true,
                &mut rng(),
            );
        }
        assert!(!state.is_active());
    }

    #[test]
    fn the_roll_never_fires_while_already_busy_with_itself() {
        let mut state = SlimeRainState {
            timer: 100,
            kill_count: 0,
            ..Default::default()
        };
        for _ in 0..500 {
            state.roll(
                false,
                true,
                true,
                1,
                false,
                true,
                false,
                true,
                true,
                &mut rng(),
            );
        }
        assert_eq!(state.timer, 100, "should never have been re-rolled");
    }

    /// Without a ready player *or* expert mode, the roll's own gate (`denominator > 0 &&
    /// (someone_ready || expert)`) refuses to fire at all — not just "less often".
    #[test]
    fn the_roll_never_fires_without_a_ready_player_or_expert_mode() {
        let mut state = SlimeRainState::default();
        for _ in 0..2000 {
            state.roll(
                false,
                true,
                true,
                1,
                false,
                true,
                false,
                false,
                false,
                &mut rng(),
            );
        }
        assert!(!state.is_active());
    }

    /// `denominator`'s own arithmetic, pinned exactly rather than only exercised indirectly — the
    /// real odds are five and six figures even at their most generous, too rare to reliably prove
    /// by actually winning enough rolls in a fast test (see the two statistical tests below for
    /// the largest realistic denominator this project's own bounds actually allow).
    #[test]
    fn the_denominator_matches_sources_own_arithmetic() {
        // A brand new world (King Slime never downed), ordinary clock rate: `450000 / 1 / 2`.
        assert_eq!(SlimeRainState::denominator(1, false, false, true), 225_000);
        // Downed, classic mode: the `/2` no longer applies and hardmode's `*1.5` does not fire.
        assert_eq!(SlimeRainState::denominator(1, true, false, true), 450_000);
        // Downed and in hardmode: `*1.5`.
        assert_eq!(SlimeRainState::denominator(1, true, true, true), 675_000);
        // Nobody ready to fight him multiplies whatever the above already is by five.
        assert_eq!(
            SlimeRainState::denominator(1, true, false, false),
            2_250_000
        );
        // Journey's fastest clock (24x) divides the base by the same twenty-four.
        assert_eq!(SlimeRainState::denominator(24, false, false, true), 9_375);
    }

    /// Expert mode alone is enough to let the roll fire, even with nobody ready — proven at the
    /// largest denominator this project's own real inputs ever produce (Journey's 24x clock,
    /// already-downed King Slime, classic mode, nobody ready) so the statistics stay tractable:
    /// `450000 / 24 * 5 = 93750`, a real number a long enough run can actually observe a win at.
    #[test]
    fn expert_mode_alone_can_still_start_a_rain() {
        let mut r = rng();
        let mut started = false;
        for _ in 0..1_000_000 {
            let mut state = SlimeRainState::default();
            state.roll(
                false, true, true, 24, false, true, false, false, true, &mut r,
            );
            if state.is_active() {
                started = true;
                break;
            }
        }
        assert!(
            started,
            "expert mode alone should have started a rain at least once"
        );
    }

    /// Before King Slime has ever been beaten, the odds are twice as generous — proven by
    /// comparing observed start rates over many independent trials rather than trusting the
    /// formula by inspection alone. Journey's 24x clock and a ready player keep the denominators
    /// (9375 before, 18750 after) small enough for the difference to show up reliably.
    #[test]
    fn the_odds_are_better_before_king_slime_has_ever_been_beaten() {
        let trials = 1_000_000;
        let starts = |downed: bool| {
            let mut r = rng();
            (0..trials)
                .filter(|_| {
                    let mut state = SlimeRainState::default();
                    state.roll(
                        false, true, true, 24, false, downed, false, true, true, &mut r,
                    );
                    state.is_active()
                })
                .count()
        };
        let before = starts(false);
        let after = starts(true);
        assert!(
            before > after,
            "before ({before}) should start more often than after ({after})"
        );
    }

    #[test]
    fn note_kill_ignores_anything_that_is_not_a_blue_slime() {
        let mut state = SlimeRainState {
            timer: 100,
            kill_count: 0,
            ..Default::default()
        };
        assert!(
            !state.note_kill(3, false, false),
            "a zombie, not a blue slime"
        );
        assert_eq!(state.kill_count, 0);
    }

    #[test]
    fn note_kill_does_nothing_while_no_rain_is_active() {
        let mut state = SlimeRainState::default();
        assert!(!state.note_kill(BLUE_SLIME, false, false));
        assert_eq!(state.kill_count, 0);
    }

    #[test]
    fn note_kill_does_nothing_while_king_slime_is_already_out() {
        let mut state = SlimeRainState {
            timer: 100,
            kill_count: 149,
            ..Default::default()
        };
        assert!(!state.note_kill(BLUE_SLIME, true, false));
        assert_eq!(
            state.kill_count, 149,
            "no count and no spawn while he is already here"
        );
    }

    #[test]
    fn one_hundred_and_fifty_kills_summons_king_slime_the_first_time() {
        let mut state = SlimeRainState {
            timer: 100,
            kill_count: 0,
            ..Default::default()
        };
        for _ in 0..149 {
            assert!(!state.note_kill(BLUE_SLIME, false, false));
        }
        assert!(
            state.note_kill(BLUE_SLIME, false, false),
            "the 150th should summon him"
        );
        assert_eq!(
            state.kill_count, -75,
            "a head start on the next one, not a full reset"
        );
    }

    #[test]
    fn once_king_slime_has_been_beaten_it_only_takes_seventy_five() {
        let mut state = SlimeRainState {
            timer: 100,
            kill_count: 0,
            ..Default::default()
        };
        for _ in 0..74 {
            assert!(!state.note_kill(BLUE_SLIME, false, true));
        }
        assert!(
            state.note_kill(BLUE_SLIME, false, true),
            "the 75th should summon him"
        );
        assert_eq!(state.kill_count, -37);
    }
}
