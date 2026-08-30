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

/// The strongest wind the world will ever blow — the strongest it *targets*, before rain drives it
/// harder still (see [`RAIN_WIND_BOOST`]).
pub const WIND_LIMIT: f32 = 0.8;
/// ...and the strongest it blows for a party who have never found a life crystal.
pub const WIND_LIMIT_EARLY: f32 = 0.35;
/// The wind never quite stops closing on its target: this is the floor on how far it moves each
/// tick (`Main.cs:59739`).
pub const WIND_APPROACH_FLOOR: f32 = 0.0003;
/// ...and on top of that floor it closes this fraction of the remaining distance every tick, so it
/// approaches fast when far and eases in when near — an exponential creep, not a fixed drift
/// (`Main.cs:59741`).
pub const WIND_APPROACH_RATE: f32 = 0.0015;
/// Rain drives the wind harder: the target it heads for is multiplied by `1 + this * maxRaining`
/// (`Main.cs:59740`), so a downpour blows past the ordinary [`WIND_LIMIT`].
pub const RAIN_WIND_BOOST: f32 = 5.0 / 9.0;
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

/// The rest of the sky state a weather tick reads, beside the wind and rain it keeps itself: two
/// events that gate weather (L3-19) and the cloud count that sets a shower's strength (L3-18).
#[derive(Debug, Clone, Copy, Default)]
pub struct Sky {
    /// Whether a lantern night is up — it holds the wind's target and stops (or forestalls) rain.
    pub lantern_night: bool,
    /// Whether the *next* night is already a guaranteed lantern night — `LanternNight.
    /// NextNightIsLanternNight`. Rain will not begin the evening before one (m3), matching vanilla's
    /// third rain-start condition at `Main.cs:65870`.
    pub next_night_is_lantern_night: bool,
    /// Whether a slime rain is on — rain will not begin over one.
    pub slime_rain: bool,
    /// How many clouds the sky is holding, which decides how hard a shower comes down.
    pub num_clouds: u16,
}

/// How hard a shower comes down, rolled from how cloudy the sky already is — `Main.ChangeRain`'s
/// own three cloud-dependent branches (`Main.cs:65710`). The `cloudBGActive >= 1` half of vanilla's
/// top branch is not modelled (this project does not track the visual cloud alpha), so only the
/// `numClouds` thresholds select the branch; that is a narrowing, not a different formula.
fn roll_rain_strength(num_clouds: u16, rng: &mut SmallRng) -> f32 {
    // Two rolls in three take the common (narrower, stronger) range; the last takes the wider one.
    let common = rng.random_range(0..3) != 0;
    let range = if num_clouds > 150 {
        if common { 40..=90 } else { 20..=90 }
    } else if num_clouds > 100 {
        if common { 20..=60 } else { 10..=70 }
    } else if common {
        5..=30
    } else {
        5..=40
    };
    rng.random_range(range) as f32 * 0.01
}

impl Weather {
    /// One tick. `strong_enough` is whether anyone in the world has found a life crystal, which
    /// is the gate on real weather happening at all.
    ///
    /// `freeze_wind`/`freeze_rain` are Journey mode's `FreezeWindDirectionAndStrength`/
    /// `FreezeRainPower` — independent in real vanilla (`Main.cs:59762` and `Main.cs:65846` gate
    /// separate update calls), which is why they are two parameters here rather than one shared
    /// pause: freezing a storm mid-downpour should not also freeze which way the wind is blowing.
    /// Sandstorms have no Journey power of their own and are never frozen by either flag.
    ///
    /// `sky` carries the rest of what vanilla's own weather reads: whether a lantern night is up,
    /// whether the next night is a guaranteed one, and whether a slime rain is on (all three gate
    /// the rain, L3-19 and m3), and how many clouds the sky is holding, which sets how hard a shower
    /// comes down (L3-18).
    pub fn tick(
        &mut self,
        strong_enough: bool,
        hard_mode: bool,
        freeze_wind: bool,
        freeze_rain: bool,
        sky: Sky,
        rng: &mut SmallRng,
    ) {
        if !freeze_wind {
            self.tick_wind(strong_enough, sky.lantern_night, rng);
        }
        if !freeze_rain {
            self.tick_rain(strong_enough, sky, rng);
        }
        self.tick_sandstorm(hard_mode, rng);
    }

    fn tick_wind(&mut self, strong_enough: bool, lantern_night: bool, rng: &mut SmallRng) {
        // L3-19: a lantern night holds the wind's target where it is (`Main.cs:59764`). The wind
        // still creeps toward that frozen target below; it only stops picking a *new* one.
        if !lantern_night {
            self.tick_wind_target(strong_enough, rng);
        }
        self.target = self.target.clamp(-WIND_LIMIT, WIND_LIMIT);

        // L3-13: the wind closes on its target exponentially (fast when far, easing in when near,
        // never quite still), toward a target rain drives harder — `Main.cs:59738-59756`, not the
        // fixed drift this used, which approached several times too fast and ignored the rain.
        let effective = self.target * (1.0 + RAIN_WIND_BOOST * self.max_rain);
        let step = WIND_APPROACH_FLOOR + (effective - self.wind).abs() * WIND_APPROACH_RATE;
        if self.wind < effective {
            self.wind = (self.wind + step).min(effective);
        } else if self.wind > effective {
            self.wind = (self.wind - step).max(effective);
        }
    }

    /// Roll a new wind target, once its counter has run out. Split from the approach so a lantern
    /// night can freeze the one without freezing the other.
    fn tick_wind_target(&mut self, strong_enough: bool, rng: &mut SmallRng) {
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

    fn tick_rain(&mut self, strong_enough: bool, sky: Sky, rng: &mut SmallRng) {
        if self.raining {
            // L3-19: a lantern night stops the rain outright (`Main.cs:65848-65851`).
            if sky.lantern_night {
                self.stop_rain();
                return;
            }
            self.rain_time -= 1;
            if self.rain_time <= 0 {
                self.stop_rain();
                return;
            }
            // L3-18: one tick in 7,200 the intensity is re-rolled mid-storm (`Main.cs:65862-65865`,
            // `86400 / 24 * 2`), so a shower is not one flat strength start to finish.
            if rng.random_range(0..7200) == 0 {
                self.max_rain = roll_rain_strength(sky.num_clouds, rng);
            }
            return;
        }
        if !strong_enough {
            return;
        }
        // L3-19 + m3: rain does not begin during a slime rain, a lantern night, or the evening
        // before a guaranteed lantern night (`Main.cs:65870`: `!slimeRain && !LanternsUp &&
        // !NextNightIsLanternNight`). The third term was the undisclosed narrowing m3 flagged.
        if sky.slime_rain || sky.lantern_night || sky.next_night_is_lantern_night {
            return;
        }
        // Roughly one shower every five and three-quarter days.
        if rng.random_range(0..(DAY as f32 * 5.75) as i32) == 0 {
            self.start_rain(sky.num_clouds, rng);
        }
    }

    /// Begin a shower: somewhere from eight hours to a day of it, at a strength set by how cloudy
    /// the sky already is (L3-18).
    pub fn start_rain(&mut self, num_clouds: u16, rng: &mut SmallRng) {
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
        self.max_rain = roll_rain_strength(num_clouds, rng);
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
            weather.tick(strong_enough, false, false, false, Sky::default(), &mut rng);
            trace.push(weather.wind);
        }
        (weather, trace)
    }

    /// The strongest the wind can ever reach: its ordinary limit, driven harder by the heaviest
    /// rain (`max_rain` tops out at 0.40 in [`Weather::start_rain`]) — the rain-amplified target
    /// from L3-13.
    const AMPLIFIED_LIMIT: f32 = WIND_LIMIT * (1.0 + RAIN_WIND_BOOST * 0.40);

    /// The wind actually moves, and never past its limit — the ordinary one in the dry, and the
    /// rain-amplified one when it is pouring.
    #[test]
    fn the_wind_blows_and_stays_within_bounds() {
        let (_, trace) = run(true, 200_000, 1);
        let strongest = trace.iter().cloned().fold(0.0f32, |a, b| a.max(b.abs()));
        assert!(strongest > 0.2, "it should blow at all: {strongest}");
        assert!(
            trace.iter().all(|w| w.abs() <= AMPLIFIED_LIMIT + 1e-6),
            "past the amplified limit: {strongest}"
        );
    }

    /// It creeps rather than jumping: no single tick moves it more than one exponential step, the
    /// floor plus the rate times the farthest it could ever be from its target (L3-13). Vanilla's
    /// approach is not the old fixed drift, so a step can be a little larger when far from target,
    /// but it is still a creep, never a teleport.
    #[test]
    fn the_wind_never_jumps() {
        let max_step = WIND_APPROACH_FLOOR + WIND_APPROACH_RATE * 2.0 * AMPLIFIED_LIMIT;
        let (_, trace) = run(true, 100_000, 2);
        for pair in trace.windows(2) {
            assert!(
                (pair[1] - pair[0]).abs() <= max_step + 1e-6,
                "the wind jumped from {} to {}",
                pair[0],
                pair[1]
            );
        }
    }

    /// L3-13: the wind closes on its target by an exponential step (fast far, slow near) rather
    /// than a fixed drift, and rain drives the target past the ordinary limit.
    ///
    /// Fails before the fix: the old approach moved a constant `0.0025` a tick, so the first step
    /// would be far larger than the exponential floor-plus-fraction, the step would not shrink as
    /// it closed in, and rain would never carry the wind past `WIND_LIMIT`.
    #[test]
    fn wind_approaches_exponentially_and_rain_drives_it_harder() {
        let mut rng = SmallRng::seed_from_u64(0);
        // A fixed target (a huge counter keeps the nudge logic from re-rolling it), no rain.
        let mut w = Weather {
            target: 0.35,
            counter: 1_000_000,
            ..Default::default()
        };
        let before = w.wind;
        w.tick_wind(true, false, &mut rng);
        let first_step = w.wind - before;
        let expected = WIND_APPROACH_FLOOR + WIND_APPROACH_RATE * 0.35;
        assert!(
            (first_step - expected).abs() < 1e-6,
            "first step {first_step} should be the exponential {expected}, not a fixed drift"
        );
        // Closing in, the step shrinks toward the floor — a fixed drift never would.
        for _ in 0..2_000 {
            w.tick_wind(true, false, &mut rng);
        }
        let near = w.wind;
        w.tick_wind(true, false, &mut rng);
        assert!(
            w.wind - near < first_step,
            "the step should shrink as the wind nears its target"
        );

        // Rain drives the target past the ordinary limit.
        let mut w = Weather {
            target: WIND_LIMIT,
            counter: 1_000_000,
            max_rain: 0.40,
            ..Default::default()
        };
        for _ in 0..20_000 {
            w.tick_wind(true, false, &mut rng);
        }
        assert!(
            w.wind > WIND_LIMIT + 0.05,
            "rain should push the wind well past {WIND_LIMIT}: {}",
            w.wind
        );
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
        // A calm world never gets one, however long it waits.
        let mut weather = Weather {
            wind: 0.1,
            target: 0.1,
            ..Default::default()
        };
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
        weather.start_rain(0, &mut rng);
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
            weather.tick(true, false, false, false, Sky::default(), &mut rng);
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
                weather.tick(true, false, false, false, Sky::default(), &mut rng);
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

    /// L3-18: a shower's intensity re-rolls mid-storm rather than holding one value throughout
    /// (`Main.cs:65862-65865`).
    ///
    /// Fails before the fix: `max_rain` was set once in `start_rain` and never touched again, so a
    /// storm came down at exactly one strength from beginning to end.
    #[test]
    fn rain_intensity_re_rolls_mid_storm() {
        let mut rng = SmallRng::seed_from_u64(3);
        let mut weather = Weather::default();
        weather.start_rain(0, &mut rng);
        let initial = weather.max_rain;
        let sky = Sky::default();
        let mut changed = false;
        while weather.raining {
            weather.tick_rain(true, sky, &mut rng);
            if (weather.max_rain - initial).abs() > 1e-6 {
                changed = true;
                break;
            }
        }
        assert!(
            changed,
            "the intensity should have re-rolled during the storm"
        );
    }

    /// L3-19: a lantern night stops rain outright (`Main.cs:65848-65851`).
    ///
    /// Fails before the fix: `tick_rain` had no notion of a lantern night, so the rain ran on.
    #[test]
    fn a_lantern_night_stops_the_rain() {
        let mut rng = SmallRng::seed_from_u64(4);
        let mut weather = Weather::default();
        weather.start_rain(0, &mut rng);
        assert!(weather.raining);
        let sky = Sky {
            lantern_night: true,
            ..Default::default()
        };
        weather.tick_rain(true, sky, &mut rng);
        assert!(
            !weather.raining,
            "a lantern night should have stopped the rain"
        );
    }

    /// L3-19: rain does not begin during a slime rain (`Main.cs:65870`).
    ///
    /// Fails before the fix: rain start was ungated, so a slime rain and an ordinary shower could
    /// run at once.
    #[test]
    fn rain_does_not_start_during_a_slime_rain() {
        let mut rng = SmallRng::seed_from_u64(5);
        let mut weather = Weather::default();
        let sky = Sky {
            slime_rain: true,
            ..Default::default()
        };
        for _ in 0..2_000_000 {
            weather.tick_rain(true, sky, &mut rng);
        }
        assert!(
            !weather.raining,
            "rain should never begin during a slime rain"
        );
    }

    /// m3: rain does not begin the evening before a guaranteed lantern night — vanilla's third
    /// rain-start condition `!NextNightIsLanternNight` (`Main.cs:65870`).
    ///
    /// Fails before the fix: the gate checked only `slime_rain || lantern_night`, so the server
    /// could open a shower the evening before a scheduled lantern night when vanilla would not. The
    /// same seeds `it_rains_of_its_own_accord` proves rain on with an open sky are held dry here by
    /// the new term, which is what makes this a genuine fail-then-pass rather than a bad-seed pass.
    #[test]
    fn rain_does_not_start_the_evening_before_a_lantern_night() {
        let sky = Sky {
            next_night_is_lantern_night: true,
            ..Default::default()
        };
        for seed in 0..8u64 {
            let mut rng = SmallRng::seed_from_u64(seed);
            let mut weather = Weather::default();
            for _ in 0..2_000_000 {
                weather.tick_rain(true, sky, &mut rng);
                assert!(
                    !weather.raining,
                    "rain should never begin before a guaranteed lantern night (seed {seed})"
                );
            }
        }
    }

    /// L3-19: a lantern night holds the wind's target where it is (`Main.cs:59764`), though the
    /// wind still creeps toward it.
    ///
    /// Fails before the fix: the wind picked a new target on its schedule regardless of the night.
    #[test]
    fn a_lantern_night_freezes_the_wind_target() {
        let mut rng = SmallRng::seed_from_u64(6);
        // A counter of zero would re-roll the target immediately on an ordinary night.
        let mut weather = Weather {
            target: 0.5,
            counter: 0,
            ..Default::default()
        };
        for _ in 0..100 {
            weather.tick_wind(true, true, &mut rng);
        }
        assert_eq!(
            weather.target, 0.5,
            "the target should be held through a lantern night"
        );
        assert!(
            weather.wind > 0.0,
            "but the wind should still creep toward it"
        );
    }
}
