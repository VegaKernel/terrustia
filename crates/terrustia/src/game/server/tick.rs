//! The game loop: the actor's `select!`, the sixty-hertz tick, and what a tick cost.
//!
//! [`GameServer::run`] owns the loop itself; [`GameServer::tick`] is one frame of the world, which
//! does nothing on its own beyond calling each system in the order vanilla does and timing it. The
//! measurement types live here too, next to the only code that reads them.

use std::time::{Duration, Instant};

use tokio::{
    sync::mpsc,
    time::{MissedTickBehavior, interval},
};
use tracing::{debug, error, info, warn};

use crate::game::clock;

use super::{GameServer, SYNC_FULL, SYNC_STREAM, ServerEvent, Stopped};

/// Vanilla runs at 60 ticks per second and the clock packets assume it.
pub(super) const TICK: Duration = Duration::from_nanos(16_666_667);
/// Ticks in a second, for turning the tick counter into a human uptime on the status footer.
const TICKS_PER_SECOND: u64 = 60;
/// How often the live status footer is refreshed — about once a second, off the tick counter.
const STATUS_EVERY: u64 = 60;

/// How often the worst tick in the window is reported, when it is worth reporting.
const TICK_REPORT_EVERY: u64 = 600;

/// The parts of a tick, in the order they run.
///
/// What used to be one `World` phase was thirteen separate systems sharing a lap, so a warning
/// saying `phase=world` narrowed the cause down to "somewhere in most of the tick". A two-hour
/// idle run reported that phase eating half the budget with two NPCs and nobody connected; the
/// cause turned out to be the autosave's world copy, which is now its own entry and would have
/// been obvious from the first warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    /// Copying the world for the background save. Runs on the tick, once every autosave.
    Snapshot,
    Liquids,
    Growth,
    Spread,
    Weather,
    /// The clock, tile entities, wiring timers, lunar events and the biome census.
    World,
    Sections,
    Items,
    Npcs,
    Projectiles,
    Damage,
    Spawning,
    Housing,
    Sync,
}

impl Phase {
    pub(super) const NAMES: [&'static str; 14] = [
        "snapshot",
        "liquids",
        "growth",
        "spread",
        "weather",
        "world",
        "sections",
        "items",
        "npcs",
        "projectiles",
        "damage",
        "spawning",
        "housing",
        "sync",
    ];
}

/// Times one phase of a tick, on the same clock the tick's own total uses.
///
/// A named type rather than two lines inline, because those two lines were wrong for months and
/// nothing could see it: phases were timed with `Instant` while the tick total came from
/// `clock::Cpu`, so the warning line compared wall microseconds against CPU microseconds and could
/// report a phase costing more than the whole tick containing it. Every phase figure ever logged
/// was inflated by however long that phase spent descheduled.
///
/// Wrapping it makes the mistake unavailable — there is nowhere here to put an `Instant` — and it
/// makes the property that matters testable on its own, which is the part that counts. Asserting
/// "no phase exceeds its tick" does *not* catch this: on an idle machine the two clocks agree, so
/// that assertion passes against the broken code, which is exactly how it survived so long.
struct PhaseClock(clock::Cpu);

impl PhaseClock {
    fn start() -> Self {
        Self(clock::Cpu::now())
    }

    /// Processor time since the last lap.
    fn lap(&mut self) -> Duration {
        let now = clock::Cpu::now();
        let elapsed = now.since(self.0);
        self.0 = now;
        elapsed
    }
}

/// Where one tick's time went.
///
/// `cpu` is what the tick cost; `wall` is how long it took to happen. They differ by however long
/// the OS gave this core to something else, which on a machine that is also running the game can
/// be tens of milliseconds. Keeping them apart is what stops a busy laptop from being reported as
/// a slow server.
#[derive(Debug, Default, Clone, Copy)]
pub(super) struct TickCost {
    pub(super) cpu: Duration,
    pub(super) wall: Duration,
    pub(super) phases: [Duration; Phase::NAMES.len()],
}

impl TickCost {
    /// The phase that took the longest, which is the one worth naming in a warning.
    fn worst_phase(&self) -> (&'static str, Duration) {
        self.phases
            .iter()
            .enumerate()
            .max_by_key(|(_, d)| **d)
            .map_or(("none", Duration::ZERO), |(i, d)| (Phase::NAMES[i], *d))
    }
}


impl GameServer {
    /// Refresh the live status footer: who is online, how long the server has been up, and the last
    /// tick's cost. Called about once a second from [`Self::note_tick_cost`]. Cheap, and a no-op on
    /// screen when there is no interactive prompt to sit above (a piped or service console).
    fn update_status(&self, cost: TickCost) {
        let p = self.palette;
        let online = self
            .players
            .iter()
            .flatten()
            .filter(|player| player.is_playing())
            .count();
        // Uptime off the tick counter rather than a wall clock: this is a health read, not a
        // timestamp, and one that never has to reach for `Instant::now` on the hot path.
        let secs = self.ticks / TICKS_PER_SECOND;
        let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
        let tick_us = cost.cpu.as_micros();
        use crate::term::sgr;
        let dot_colour = if online > 0 {
            sgr::BRIGHT_GREEN
        } else {
            sgr::DIM
        };
        let status = format!(
            "  {} {} online   {}   {}",
            p.paint(dot_colour, "●"),
            p.paint(sgr::BOLD, &online.to_string()),
            p.paint(sgr::DIM, &format!("up {h:02}:{m:02}:{s:02}")),
            p.paint(sgr::DIM, &format!("tick {tick_us}µs")),
        );
        crate::term::set_status(&status);
    }

    pub async fn run(mut self, mut events: mpsc::Receiver<ServerEvent>) -> Stopped {
        // Whoever lived here when the world was last saved lives here again.
        self.restore_town_npcs();
        self.announce_claim_token();

        let mut ticker = interval(TICK);
        // Catching up on missed ticks would fast-forward the world clock after any stall.
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut outcome = Stopped::Cleanly;
        loop {
            tokio::select! {
                event = events.recv() => match event {
                    // Wrapped for the same reason the tick below is, and more urgently: this is
                    // the path every byte from an untrusted client travels. It was left bare, so
                    // a panic anywhere under `handle_packet` — or in any of the ~130 AI routines
                    // beneath it — unwound straight out of this loop, past the shutdown save at
                    // the bottom of the function, taking everything since the last autosave.
                    Some(event) => {
                        let handled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                            || self.handle_event(event),
                        ));
                        if handled.is_err() {
                            error!("handling a packet panicked; saving the world and stopping");
                            outcome = Stopped::Panicked;
                            break;
                        }
                    }
                    None => break,
                },
                _ = ticker.tick() => {
                    // A panic in here would otherwise take the world with it. The game is a
                    // single task and the shutdown save below lives inside it, so an unwind
                    // straight out of the loop loses everything since the last autosave. Catching
                    // it turns that into a clean stop that still writes the world out.
                    //
                    // `AssertUnwindSafe` is the honest choice rather than a safe one: the server's
                    // state may well be inconsistent after a panic. That is exactly why this saves
                    // and stops rather than carrying on.
                    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.tick())) {
                        Ok(cost) => {
                            self.note_tick_cost(cost);
                            if self.stopping {
                                break;
                            }
                        }
                        Err(_) => {
                            error!("the game loop panicked; saving the world and stopping");
                            outcome = Stopped::Panicked;
                            break;
                        }
                    }
                }
            }
        }

        // The channel closing is the shutdown signal, so this is the last chance to persist.
        //
        // Let a background save finish first if one is in flight. Both write through a temporary
        // file and rename, so neither can leave a half-written world — but the shutdown save has
        // the newer state and must land last, and two renames racing would decide that by
        // scheduling rather than by which is newer.
        if let Some(running) = self.saving.take() {
            let _ = running.await;
        }
        self.save_world("shutdown");
        info!("game loop stopped");
        outcome
    }

    /// Keep an eye on how much of the sixteen-millisecond budget a tick is actually using.
    ///
    /// A server that is quietly overrunning its budget looks identical to one that is not, right
    /// up until the world starts running slow. Reporting the worst tick in each ten-second window,
    /// and only when it is over half the budget, makes that visible without a line a second.
    ///
    /// Two different problems can push a tick over its budget and they need different answers, so
    /// they get different messages: work that costs too much processor is this server's bug, and a
    /// tick that took a long time without using the processor is the machine being busy elsewhere.
    /// The breakdown comes with the first one, because "a tick took 26 ms" is a mystery and "the
    /// spawn scan took 26 ms" is a bug report.
    fn note_tick_cost(&mut self, cost: TickCost) {
        self.last_tick = cost;
        if self.ticks.is_multiple_of(STATUS_EVERY) {
            self.update_status(cost);
        }
        if cost.cpu > self.worst_tick.cpu {
            self.worst_tick = cost;
        }
        self.worst_stall = self.worst_stall.max(cost.wall.saturating_sub(cost.cpu));
        if !self.ticks.is_multiple_of(TICK_REPORT_EVERY) {
            return;
        }
        let worst = std::mem::take(&mut self.worst_tick);
        let stall = std::mem::take(&mut self.worst_stall);
        debug!(
            cpu_us = worst.cpu.as_micros() as u64,
            wall_us = worst.wall.as_micros() as u64,
            stall_us = stall.as_micros() as u64,
            phase = worst.worst_phase().0,
            npcs = self.npcs.len(),
            sync_full = SYNC_FULL.load(std::sync::atomic::Ordering::Relaxed),
            sync_stream = SYNC_STREAM.load(std::sync::atomic::Ordering::Relaxed),
            "tick window"
        );
        if worst.cpu * 2 > TICK {
            let (phase, phase_cost) = worst.worst_phase();
            warn!(
                worst_us = worst.cpu.as_micros() as u64,
                budget_us = TICK.as_micros() as u64,
                phase,
                phase_us = phase_cost.as_micros() as u64,
                npcs = self.npcs.len(),
                projectiles = self.projectiles.len(),
                "ticks are using a lot of their budget"
            );
        } else if stall > TICK * 6 {
            // Not a warning: nothing here is wrong, the machine is just busy. The threshold is six
            // ticks (~100 ms) rather than one, because a single-tick stall is a dropped frame nobody
            // notices, and an idle laptop that naps for a moment should not narrate it. A stall this
            // size is a real hitch a player feels, and worth one quiet line per ten-second window.
            info!(
                stall_us = stall.as_micros() as u64,
                cpu_us = worst.cpu.as_micros() as u64,
                "the game loop was held off the processor; the machine is busy elsewhere"
            );
        }
    }

    pub(super) fn tick(&mut self) -> TickCost {
        let mut cost = TickCost::default();
        let began = Instant::now();
        let cpu_began = clock::Cpu::now();
        // Phases are timed on the *same* clock as the tick total, which they were not: the total
        // came from `clock::Cpu` and the laps from `Instant`, so the warning line compared CPU
        // microseconds against wall microseconds and could report a phase costing more than the
        // whole tick that contained it. Every phase figure ever logged was inflated by however
        // long that phase spent descheduled. Nine extra thread-clock reads a tick is nothing —
        // it is a vDSO call — and it makes the phases add up to the total, which is the only way
        // the breakdown means anything.
        let mut clock = PhaseClock::start();
        let mut lap = |cost: &mut TickCost, phase: Phase| {
            cost.phases[phase as usize] += clock.lap();
        };

        self.ticks += 1;
        let was_day = self.world.day_time;
        // Journey mode's `FreezeTime` (`Main.cs:6342` gates the whole day/night update the same
        // way). The clock — and everything below keyed off it turning midnight or dawn — simply
        // does not run this tick; nothing here needs its own separate "and skip that too" branch.
        // `ModifyTimeRate` (`Main.cs:6343`'s own `targetTimeRate`) is the other half of the same
        // gate in source — applied here as the tick count itself rather than a separate branch,
        // since `tick_time`'s own loop already handles more than one day/night flip in one call.
        if !self.journey.freeze_time {
            self.world.tick_time(self.journey.time_rate());
            self.tick_slime_rain();
        }
        // Dawn puts the moons away and takes the blood moon with them, and rolls for an eclipse.
        if self.world.day_time && !was_day {
            self.stop_moon();
            self.world.blood_moon = false;
            self.roll_dawn_events();
            self.broadcast_world_data();
        }
        // Dusk rolls for a blood moon, which needs somebody with more than a hundred and twenty
        // life to be worth having.
        if !self.world.day_time && was_day {
            self.roll_dusk_events();
        }
        if !self.world.day_time && was_day && self.world.eclipse {
            self.world.eclipse = false;
            self.announce("The solar eclipse is over.");
            self.broadcast_world_data();
        }
        self.tick_party();

        if let Some(every) = self.autosave_ticks
            && self.ticks.is_multiple_of(every)
        {
            self.save_world_in_background("autosave");
        }
        // Its own phase because it is the single most expensive thing the tick does, and it was
        // hidden inside a bucket of thirteen systems.
        lap(&mut cost, Phase::Snapshot);
        self.note_finished_save();
        self.note_finished_auth();
        self.reclaim_snapshot_buffer();
        self.tick_tile_spam();
        // What the world is worth fighting at, refreshed before anything can spawn. Cheap, and
        // keeping it here means no spawn site has to remember to scale.
        let difficulty = self.effective_difficulty();
        self.npcs.set_scaling(crate::game::npc::Scaling {
            difficulty,
            players: self
                .players
                .iter()
                .flatten()
                .filter(|p| p.is_playing())
                .count() as u32,
        });
        self.projectiles.set_hostile_damage_scale(
            terrustia_proto::difficulty::hostile_projectile_multiplier(difficulty),
        );

        self.tick_liquids();
        lap(&mut cost, Phase::Liquids);
        self.tick_growth();
        lap(&mut cost, Phase::Growth);
        self.tick_spread();
        lap(&mut cost, Phase::Spread);
        self.tick_weather();
        lap(&mut cost, Phase::Weather);
        // Whatever is left: the tile entities, the mech cooldowns, the wiring timers, the lunar
        // event and the biome census. Individually small; kept together so the breakdown does not
        // become a wall of near-zero lines.
        self.tick_tile_entities();
        self.tick_mech_cooldowns();
        self.tick_timers();
        self.tick_lunar();
        self.tick_census();
        lap(&mut cost, Phase::World);

        self.flush_dirty_sections();
        self.drain_section_streams();
        lap(&mut cost, Phase::Sections);
        self.tick_items();
        lap(&mut cost, Phase::Items);
        self.tick_npc_buffs();
        self.tick_npcs();
        lap(&mut cost, Phase::Npcs);
        self.tick_projectiles();
        lap(&mut cost, Phase::Projectiles);
        self.tick_contact_damage();
        lap(&mut cost, Phase::Damage);
        self.tick_spawning();
        lap(&mut cost, Phase::Spawning);
        self.tick_town_npcs();
        self.tick_travelling_merchant();
        self.tick_old_man();
        self.tick_cultist_tablet();
        lap(&mut cost, Phase::Housing);

        lap(&mut cost, Phase::Sync);

        cost.cpu = clock::Cpu::now().since(cpu_began);
        cost.wall = began.elapsed();
        cost
    }

}

/// Journey mode's `FreezeTime` actually stops the clock — not just the toggle sticking, the real
/// gameplay effect (`tick()`'s own gate on `self.journey.freeze_time`, mirroring `Main.cs:6342`'s
/// gate on the same power in source).
#[cfg(test)]
mod freeze_time {
    use super::*;
    use crate::config::Config;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "freeze time probe")
    }

    #[test]
    fn frozen_time_does_not_advance_across_many_ticks() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.journey.freeze_time = true;
        let (day_time, time) = (server.world.day_time, server.world.time);

        for _ in 0..500 {
            server.tick();
        }

        assert_eq!(
            (server.world.day_time, server.world.time),
            (day_time, time),
            "the clock should not have moved a single tick while frozen"
        );
    }

    #[test]
    fn unfreezing_lets_it_advance_again() {
        let mut server = GameServer::new(Config::default(), tiny_world());
        server.journey.freeze_time = true;
        let before = server.world.time;
        for _ in 0..10 {
            server.tick();
        }
        assert_eq!(server.world.time, before, "still frozen so far");

        server.journey.freeze_time = false;
        for _ in 0..10 {
            server.tick();
        }
        assert!(
            server.world.time > before,
            "the clock should have moved once unfrozen, got {} from a start of {before}",
            server.world.time
        );
    }
}

/// Journey mode's `ModifyTimeRate` actually changes how fast the clock runs — `tick()`'s own
/// `self.journey.time_rate()` argument to `tick_time`, mirroring `Main.cs:6343`'s own
/// `targetTimeRate` read in source.
#[cfg(test)]
mod modify_time_rate {
    use super::*;
    use crate::config::Config;

    fn tiny_world() -> crate::world::World {
        crate::world::World::empty(200, 150, "time rate probe")
    }

    #[test]
    fn the_top_of_the_slider_advances_the_clock_twenty_four_times_as_fast() {
        let mut baseline = GameServer::new(Config::default(), tiny_world());
        let mut sped_up = GameServer::new(Config::default(), tiny_world());
        sped_up.journey.time_rate_slider = 1.0; // the slider's real top: 24x

        // Deltas, not absolute values: `new()`'s own startup work (angler quest roll and friends)
        // can leave `world.time` non-zero before the first real tick, which a bare before/after-one-
        // tick comparison would otherwise fold into the "24x" ratio and make it come out wrong.
        let (before_baseline, before_sped) = (baseline.world.time, sped_up.world.time);
        baseline.tick();
        sped_up.tick();
        let (moved_baseline, moved_sped) = (
            baseline.world.time - before_baseline,
            sped_up.world.time - before_sped,
        );

        assert_eq!(
            moved_baseline, 1,
            "an ordinary tick should move the clock by exactly one"
        );
        assert_eq!(
            moved_sped, 24,
            "one tick at the slider's top should move the clock 24 real ticks' worth"
        );
    }
}

/// Do the tick's phases and its total actually describe the same thing?
///
/// They did not. `worst_us` came from `clock::Cpu` and `phase_us` from `Instant`, so the warning
/// line compared CPU microseconds against wall microseconds. A real two-hour run logged three
/// ticks where the phase cost *more than the whole tick containing it* — which is impossible, and
/// meant every phase figure was inflated by however long that phase spent descheduled. All of
/// Stage 2's measurement rests on these numbers, so the invariant is pinned here.
#[cfg(test)]
mod tick_accounting {
    use super::*;
    use crate::config::Config;

    #[tokio::test]
    async fn no_phase_can_cost_more_than_its_own_tick() {
        let mut server = GameServer::new(Config::default(), World::empty(600, 400, "accounting"));

        for _ in 0..20 {
            let cost = server.tick();
            let (name, worst) = cost.worst_phase();
            assert!(
                worst <= cost.cpu,
                "phase {name} cost {worst:?} of a tick that cost {:?} — the two are being \
                 measured on different clocks again",
                cost.cpu
            );

            // And the parts must add up to the whole, not merely each be smaller than it.
            let summed: Duration = cost.phases.iter().sum();
            assert!(
                summed <= cost.cpu,
                "the phases sum to {summed:?} but the tick cost {:?}",
                cost.cpu
            );
        }
    }

    /// Wall clock is still recorded separately, because telling "we are slow" from "the machine
    /// is busy" is the reason this instrumentation exists at all.
    #[tokio::test]
    async fn wall_clock_is_still_measured_apart_from_processor_time() {
        let mut server = GameServer::new(Config::default(), World::empty(300, 200, "accounting"));
        let cost = server.tick();
        assert!(
            cost.wall >= cost.cpu,
            "a tick cannot use more processor than it took: cpu {:?}, wall {:?}",
            cost.cpu,
            cost.wall
        );
    }

    /// Every phase has a name, so a breakdown can never print an index.
    #[test]
    fn every_phase_is_named() {
        assert_eq!(Phase::NAMES.len(), Phase::Sync as usize + 1);
    }

    /// The property the fix actually turns on: time spent off the processor is not phase time.
    ///
    /// This is the test that catches the bug, and the reason the two above do not. On an idle
    /// machine wall clock and CPU clock agree, so "no phase exceeds its tick" passes happily
    /// against the broken code — verified by reverting the fix and watching it stay green.
    /// Sleeping forces the two clocks apart on purpose, which is the only reliable way to tell
    /// them apart without a loaded machine.
    #[test]
    fn a_phase_does_not_charge_for_time_spent_descheduled() {
        let mut clock = PhaseClock::start();
        std::thread::sleep(Duration::from_millis(40));
        let charged = clock.lap();
        assert!(
            charged < Duration::from_millis(5),
            "a phase that slept for 40ms was charged {charged:?}; phases are on the wall clock \
             again, which inflates every figure the breakdown prints"
        );
    }

    /// And it does still charge for work, so the clock is not simply stuck at zero.
    #[test]
    fn a_phase_does_charge_for_work() {
        let mut clock = PhaseClock::start();
        let mut total = 0u64;
        for i in 0..4_000_000u64 {
            total = total.wrapping_add(i * i);
        }
        std::hint::black_box(total);
        assert!(
            clock.lap() > Duration::ZERO,
            "four million multiplies cost nothing?"
        );
    }
}

