//! Wind and rain.
//!
//! Both are simple simulations that several ported routines already read and have been reading
//! nothing but calm from. Wind matters: a windy day is what makes a kite fly, a tumbleweed roll,
//! and a dandelion's seeds carry, and one of them will not appear at all in still air. Rain
//! matters too — town NPCs go indoors for it, and it widens the surface spawn pool.
//!
//! Neither drifts smoothly. The wind holds a target for anywhere from fifteen seconds to
//! three-quarters of a minute and then jumps to a new one, and every so often it rolls a much
//! larger jump than usual — which is where a genuinely windy day comes from rather than a slightly
//! breezy one. And it will not go past a third of full strength at all until somebody in the world
//! has found a life crystal.

use rand::{Rng, rngs::SmallRng};

/// The strongest wind the world will ever blow.
pub const WIND_LIMIT: f32 = 0.8;
/// ...and the strongest it blows for a party who have never found a life crystal.
pub const WIND_LIMIT_EARLY: f32 = 0.35;
/// How fast the wind actually gets to where it is going.
pub const WIND_DRIFT: f32 = 0.0025;
/// A day counts as windy past this, which is what several routines are actually asking.
pub const WINDY: f32 = 0.4;

/// The weather as the world keeps it between ticks.
#[derive(Debug, Clone, Copy)]
pub struct Weather {
    /// What the wind is doing right now.
    pub wind: f32,
    /// What it is heading toward.
    pub target: f32,
    /// Ticks until it picks a new target.
    pub counter: i32,
    /// How many more ordinary rolls before it takes a much bigger one.
    pub extreme_counter: i32,
    pub raining: bool,
    /// Ticks of rain left.
    pub rain_time: i32,
    /// How hard it is coming down, nought to one.
    pub max_rain: f32,
    /// Whether a sandstorm is blowing, and how long it has left.
    ///
    /// A sandstorm is not weather of its own — it is what a strong enough wind does over a desert,
    /// and it dies when the wind drops. Nothing but the tumbleweed reads it, and the tumbleweed is
    /// a different creature in one.
    pub sandstorm: bool,
    pub sandstorm_time: i32,
    pub severity: f32,
    /// What the severity is heading toward. Kept public so a loaded world can restore it.
    pub intended_severity: f32,
}

impl Default for Weather {
    fn default() -> Self {
        Self {
            wind: 0.0,
            target: 0.0,
            counter: 0,
            extreme_counter: 0,
            raining: false,
            rain_time: 0,
            max_rain: 0.0,
            sandstorm: false,
            sandstorm_time: 0,
            severity: 0.0,
            intended_severity: 0.0,
        }
    }
}

/// A day is 86,400 ticks by the game's own reckoning of weather timings.
const DAY: i32 = 86_400;

impl Weather {
    /// One tick. `strong_enough` is whether anyone in the world has found a life crystal, which
    /// is the gate on real weather happening at all.
    pub fn tick(&mut self, strong_enough: bool, hard_mode: bool, rng: &mut SmallRng) {
        self.tick_wind(strong_enough, rng);
        self.tick_rain(strong_enough, rng);
        self.tick_sandstorm(hard_mode, rng);
    }

    fn tick_wind(&mut self, strong_enough: bool, rng: &mut SmallRng) {
        self.counter -= 1;
        if self.counter <= 0 {
            let was = if self.target < 0.0 { -1.0 } else { 1.0 };
            // Three sizes of nudge, rolled in order, so most changes are small and a few are not.
            let nudge = if rng.random_range(0..4) == 0 {
                rng.random_range(-25..=25)
            } else if rng.random_range(0..2) == 0 {
                rng.random_range(-50..=50)
            } else {
                rng.random_range(-100..=100)
            };
            self.target += nudge as f32 * 0.001;
            self.cap(strong_enough);

            self.extreme_counter -= 1;
            if self.extreme_counter <= 0 {
                self.reset_counter(true, rng);
                // The big roll: mostly it settles the wind down, sometimes it opens it right up.
                if rng.random_range(0..30) < 13 {
                    if rng.random_range(0..2) == 0 {
                        self.target = 0.0;
                        // A dead calm is held for a long time — two to eight minutes.
                        self.counter = rng.random_range(7200..=28800);
                    } else {
                        self.target = rng.random_range(-200..=200) as f32 * 0.001;
                    }
                } else if rng.random_range(0..20) < 13 {
                    self.target = rng.random_range(-400..=400) as f32 * 0.001;
                } else {
                    self.target = rng.random_range(-850..=850) as f32 * 0.001;
                }
                self.cap(strong_enough);
                // A strong wind buys itself more time before the next big roll, which is what
                // makes a windy day last rather than flicker.
                let strength = self.target.abs();
                if strength > 0.3 {
                    self.extreme_counter += rng.random_range(5..=10);
                }
                if strength > 0.5 {
                    self.extreme_counter += rng.random_range(10..=20);
                }
                if strength > 0.7 {
                    self.extreme_counter += rng.random_range(15..=30);
                }
            } else {
                self.reset_counter(false, rng);
            }

            // Two turns in three, a wind that just reversed is put back the way it was — so the
            // direction is sticky even when the strength is not.
            let now = if self.target < 0.0 { -1.0 } else { 1.0 };
            if rng.random_range(0..3) != 0 && was != now {
                self.target *= -1.0;
            }
        }
        self.target = self.target.clamp(-WIND_LIMIT, WIND_LIMIT);

        // The wind itself only ever creeps toward its target.
        if self.wind < self.target {
            self.wind = (self.wind + WIND_DRIFT).min(self.target);
        } else if self.wind > self.target {
            self.wind = (self.wind - WIND_DRIFT).max(self.target);
        }
    }

    fn cap(&mut self, strong_enough: bool) {
        if !strong_enough && self.target.abs() > WIND_LIMIT_EARLY {
            self.target = WIND_LIMIT_EARLY * self.target.signum();
        }
    }

    fn reset_counter(&mut self, extreme: bool, rng: &mut SmallRng) {
        self.counter = rng.random_range(900..=2700);
        if extreme {
            self.extreme_counter = rng.random_range(10..=30);
        }
    }

    fn tick_rain(&mut self, strong_enough: bool, rng: &mut SmallRng) {
        if self.raining {
            self.rain_time -= 1;
            if self.rain_time <= 0 {
                self.stop_rain();
            }
            return;
        }
        if !strong_enough {
            return;
        }
        // Roughly one shower every five and three-quarter days.
        if rng.random_range(0..(DAY as f32 * 5.75) as i32) == 0 {
            self.start_rain(rng);
        }
    }

    /// Begin a shower: somewhere from eight hours to a day of it, at a strength of its own.
    pub fn start_rain(&mut self, rng: &mut SmallRng) {
        let hour = DAY / 24;
        let mut length = rng.random_range(hour * 8..DAY);
        // Six independent rolls that each sometimes extend it, which is what makes a long
        // downpour rare rather than merely uncommon.
        for (one_in, extra) in [(3, 1), (4, 2), (5, 2), (6, 3), (7, 4), (8, 5)] {
            if rng.random_range(0..one_in) == 0 {
                length += rng.random_range(0..hour * extra);
            }
        }
        let mut stretch = 1.0f32;
        for (one_in, by) in [(2, 0.05), (3, 0.1), (4, 0.15), (5, 0.2)] {
            if rng.random_range(0..one_in) == 0 {
                stretch += by;
            }
        }
        self.rain_time = (length as f32 * stretch) as i32;
        self.raining = true;
        self.max_rain = if rng.random_range(0..3) != 0 {
            rng.random_range(5..=30) as f32 * 0.01
        } else {
            rng.random_range(5..=40) as f32 * 0.01
        };
    }

    pub fn stop_rain(&mut self) {
        self.raining = false;
        self.rain_time = 0;
        self.max_rain = 0.0;
    }

    /// One tick of the sandstorm.
    ///
    /// It needs a real wind to begin and a real wind to continue: drop below the threshold and it
    /// bleeds away fifteen times faster than it otherwise would, and a dead calm kills it outright.
    fn tick_sandstorm(&mut self, hard_mode: bool, rng: &mut SmallRng) {
        if self.sandstorm {
            self.sandstorm_time -= 1;
            if !self.wind_enough_for_sand() {
                self.sandstorm_time -= 15;
            }
            if self.wind == 0.0 {
                self.sandstorm_time = 0;
            }
            if self.sandstorm_time <= 0 {
                self.sandstorm = false;
                self.sandstorm_time = 0;
                self.roll_severity(rng);
            }
        } else if self.wind_enough_for_sand() {
            // Twice as likely in hardmode as before it, which is what makes a desert a worse
            // place to be later than it was.
            let one_in = if hard_mode { 21_600 * 2 } else { 21_600 * 3 };
            if rng.random_range(0..one_in) == 0 {
                self.sandstorm = true;
                // Eight hours to a full day of it.
                self.sandstorm_time = rng.random_range(28_800..=86_400);
                self.roll_severity(rng);
            }
        }
        if rng.random_range(0..18_000) == 0 {
            self.roll_severity(rng);
        }
        // The severity creeps toward what it is aiming at rather than jumping.
        let toward = (self.intended_severity - self.severity).signum();
        let next = (self.severity + 0.003 * toward).clamp(0.0, 1.0);
        // Overshooting means it has arrived.
        self.severity = if (self.intended_severity - next).signum() != toward {
            self.intended_severity
        } else {
            next
        };
    }

    fn roll_severity(&mut self, rng: &mut SmallRng) {
        self.intended_severity = if self.sandstorm {
            rng.random_range(0.2..=1.0)
        } else {
            0.0
        };
    }

    /// A sandstorm needs six tenths of a full wind behind it.
    fn wind_enough_for_sand(&self) -> bool {
        self.wind.abs() >= 0.6
    }

    /// Whether it is windy enough for the things that need wind to do anything.
    pub fn windy(&self) -> bool {
        self.wind.abs() >= WINDY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use rand::SeedableRng;

    fn run(strong_enough: bool, ticks: i32, seed: u64) -> (Weather, Vec<f32>) {
        let mut rng = SmallRng::seed_from_u64(seed);
        let mut weather = Weather::default();
        let mut trace = Vec::new();
        for _ in 0..ticks {
            weather.tick(strong_enough, false, &mut rng);
            trace.push(weather.wind);
        }
        (weather, trace)
    }

    /// The wind actually moves, and never past its limit.
    #[test]
    fn the_wind_blows_and_stays_within_bounds() {
        let (_, trace) = run(true, 200_000, 1);
        let strongest = trace.iter().cloned().fold(0.0f32, |a, b| a.max(b.abs()));
        assert!(strongest > 0.2, "it should blow at all: {strongest}");
        assert!(
            trace.iter().all(|w| w.abs() <= WIND_LIMIT + 1e-6),
            "past the limit: {strongest}"
        );
    }

    /// It creeps rather than jumping: no single tick moves it more than its drift.
    #[test]
    fn the_wind_never_jumps() {
        let (_, trace) = run(true, 100_000, 2);
        for pair in trace.windows(2) {
            assert!(
                (pair[1] - pair[0]).abs() <= WIND_DRIFT + 1e-6,
                "the wind jumped from {} to {}",
                pair[0],
                pair[1]
            );
        }
    }

    /// It blows both ways — over enough worlds, not necessarily within one.
    ///
    /// The direction is deliberately sticky: two turns in three, a wind that has just reversed is
    /// put straight back the way it was. So a single world can spend a very long time blowing one
    /// way, and asking one seed for both directions is asking the wrong question.
    #[test]
    fn the_wind_blows_both_ways() {
        let (mut east, mut west) = (false, false);
        for seed in 0..12u64 {
            let (_, trace) = run(true, 200_000, seed);
            east |= trace.iter().any(|w| *w > 0.1);
            west |= trace.iter().any(|w| *w < -0.1);
        }
        assert!(east, "no world ever blew east");
        assert!(west, "no world ever blew west");
    }

    /// Until somebody has found a life crystal it will not blow hard at all.
    #[test]
    fn a_weak_world_gets_no_real_weather() {
        let (weather, trace) = run(false, 300_000, 4);
        let strongest = trace.iter().cloned().fold(0.0f32, |a, b| a.max(b.abs()));
        assert!(
            strongest <= WIND_LIMIT_EARLY + 1e-6,
            "too strong for a weak world: {strongest}"
        );
        assert!(!weather.raining, "and it should not have rained");
    }

    /// A windy day happens, and is rarer than a still one.
    #[test]
    fn windy_days_are_the_exception() {
        let (mut windy, mut total) = (0usize, 0usize);
        for seed in 0..8u64 {
            let (_, trace) = run(true, 200_000, seed);
            windy += trace.iter().filter(|w| w.abs() >= WINDY).count();
            total += trace.len();
        }
        assert!(windy > 0, "it should be windy sometimes");
        assert!(
            windy < total / 2,
            "but not most of the time: {windy} of {total}"
        );
    }

    /// A sandstorm needs a real wind, and dies when the wind drops.
    #[test]
    fn a_sandstorm_lives_and_dies_by_the_wind() {
        let mut rng = SmallRng::seed_from_u64(20);
        let mut weather = Weather::default();

        // A calm world never gets one, however long it waits.
        weather.wind = 0.1;
        weather.target = 0.1;
        for _ in 0..500_000 {
            weather.tick_sandstorm(true, &mut rng);
        }
        assert!(!weather.sandstorm, "a breeze should not raise a sandstorm");

        // A real wind eventually does.
        weather.wind = 0.7;
        let mut raised = false;
        for _ in 0..500_000 {
            weather.tick_sandstorm(true, &mut rng);
            if weather.sandstorm {
                raised = true;
                break;
            }
        }
        assert!(raised, "a strong wind should raise one");
        assert!(weather.sandstorm_time >= 28_800, "and it should last hours");

        // Dropping the wind kills it far faster than it would have died.
        let was = weather.sandstorm_time;
        weather.wind = 0.1;
        for _ in 0..100 {
            weather.tick_sandstorm(true, &mut rng);
        }
        assert!(
            weather.sandstorm_time < was - 100,
            "the wind dropping should be costing it more than time alone"
        );

        // And a dead calm ends it outright.
        weather.wind = 0.0;
        weather.tick_sandstorm(true, &mut rng);
        assert!(!weather.sandstorm, "a dead calm should have ended it");
    }

    /// Hardmode makes sandstorms half again as likely.
    #[test]
    fn hardmode_raises_more_sandstorms() {
        let count = |hard_mode: bool| {
            let mut raised = 0;
            for seed in 0..6u64 {
                let mut rng = SmallRng::seed_from_u64(seed);
                let mut weather = Weather {
                    wind: 0.7,
                    ..Default::default()
                };
                let mut was = false;
                for _ in 0..400_000 {
                    weather.tick_sandstorm(hard_mode, &mut rng);
                    if weather.sandstorm && !was {
                        raised += 1;
                    }
                    was = weather.sandstorm;
                }
            }
            raised
        };
        let before = count(false);
        let after = count(true);
        assert!(
            after > before,
            "{after} in hardmode against {before} before it"
        );
    }

    /// The severity creeps rather than jumping, and stays inside its bounds.
    #[test]
    fn the_severity_creeps() {
        let mut rng = SmallRng::seed_from_u64(21);
        let mut weather = Weather {
            wind: 0.7,
            ..Default::default()
        };
        let mut last = weather.severity;
        for _ in 0..200_000 {
            weather.tick_sandstorm(true, &mut rng);
            assert!(
                (0.0..=1.0).contains(&weather.severity),
                "severity {} is out of bounds",
                weather.severity
            );
            assert!(
                (weather.severity - last).abs() <= 0.0031,
                "severity jumped from {last} to {}",
                weather.severity
            );
            last = weather.severity;
        }
    }

    /// Rain starts, runs for hours, and stops on its own.
    #[test]
    fn rain_starts_and_stops() {
        let mut rng = SmallRng::seed_from_u64(6);
        let mut weather = Weather::default();
        weather.start_rain(&mut rng);
        assert!(weather.raining);
        assert!(weather.max_rain > 0.0 && weather.max_rain <= 0.4);
        // Eight hours at the very least, by the game's own reckoning of a day.
        assert!(
            weather.rain_time >= DAY / 3,
            "a shower should last: {}",
            weather.rain_time
        );

        let started = weather.rain_time;
        let mut ticks = 0;
        while weather.raining && ticks < started + 10 {
            weather.tick(true, false, &mut rng);
            ticks += 1;
        }
        assert!(!weather.raining, "and it should have stopped");
        assert_eq!(weather.max_rain, 0.0);
    }

    /// It rains on its own eventually, without being told to.
    #[test]
    fn it_rains_of_its_own_accord() {
        let mut rained = false;
        for seed in 0..8u64 {
            let mut rng = SmallRng::seed_from_u64(seed);
            let mut weather = Weather::default();
            for _ in 0..2_000_000 {
                weather.tick(true, false, &mut rng);
                if weather.raining {
                    rained = true;
                    break;
                }
            }
            if rained {
                break;
            }
        }
        assert!(rained, "two million ticks and never a drop");
    }
}
