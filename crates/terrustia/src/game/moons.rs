//! The pumpkin moon, the frost moon, and the solar eclipse.
//!
//! The two moons are the same machine with different rosters: twenty waves, each with its own
//! point quota and its own spawn table, and one night to get through as many as you can. There is
//! no winning them — the wave counter simply stops at twenty and dawn ends the event wherever you
//! got to. What you kill is worth points rather than one each, so a Pumpking is a third of a wave
//! by itself and a scarecrow is nothing.
//!
//! The eclipse is not a wave event at all. It is a spawn-pool swap for one day, gated on how far
//! through the game you are: Mothron only turns up once Plantera is down, and a Reaper needs all
//! three mechanical bosses.

use rand::{Rng, rngs::SmallRng};

/// Which of the three is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moon {
    Pumpkin,
    Frost,
}

/// Points to finish each wave. Index is the wave; wave twenty is the last and never completes.
pub const MOON_WAVE_POINTS: [i32; 21] = [
    0, 25, 40, 50, 80, 100, 160, 180, 200, 250, 300, 375, 450, 525, 675, 850, 1025, 1325, 1550,
    2000, 0,
];

/// The last wave. Reaching it is as far as either moon goes.
pub const MOON_LAST_WAVE: i32 = 20;

/// What killing this is worth toward the wave.
///
/// Everything not on the list is worth nothing, which is why fighting the ordinary night-time
/// spawns during a moon does not advance it at all.
pub fn moon_points(npc_type: u16) -> i32 {
    match npc_type {
        // The pumpkin moon.
        305..=314 => 1,
        315 => 50,
        325 => 75,
        326 => 2,
        327 => 150,
        329 => 5,
        330 => 10,
        // The frost moon.
        338..=340 => 1,
        341 => 20,
        342 => 2,
        343 => 18,
        344 => 50,
        345 => 150,
        346 => 100,
        347 => 8,
        348 | 349 => 4,
        350 => 3,
        351 => 10,
        352 => 5,
        _ => 0,
    }
}

/// Expert doubles what a kill is worth, master two and a half times it —
/// `NPC.GetMoonEventPointScalar`'s own `Main.masterMode`/`Main.expertMode` checks, not a raw
/// `game_mode` read: both booleans are `Difficulty >= 3`/`>= 2` in real vanilla, so a Journey
/// world's `DifficultySlider` reaches this the same way it reaches everything else that reads
/// them (`server.rs`'s `is_expert()`/`is_master()`).
pub fn moon_point_scale(expert: bool, master: bool) -> f32 {
    if master {
        2.5
    } else if expert {
        2.0
    } else {
        1.0
    }
}

/// One entry of a wave's table: roll one in `one_in`, and only if fewer than `cap` are out.
#[derive(Debug, Clone, Copy)]
struct Roll {
    one_in: u32,
    npc_type: u16,
    cap: usize,
}

const NO_CAP: usize = usize::MAX;

/// Always taken if nothing before it was.
const fn fallback(npc_type: u16) -> Roll {
    Roll {
        one_in: 1,
        npc_type,
        cap: NO_CAP,
    }
}

const fn roll(one_in: u32, npc_type: u16) -> Roll {
    Roll {
        one_in,
        npc_type,
        cap: NO_CAP,
    }
}

const fn capped(one_in: u32, npc_type: u16, cap: usize) -> Roll {
    Roll {
        one_in,
        npc_type,
        cap,
    }
}

/// A trash mob, picked from a run of types. `0` means "one of the wave's rank and file".
const TRASH: u16 = 0;

/// What comes out of a moon this spawn, given the wave and what is already on the field.
///
/// Returns `None` when the wave's table declined everything, which happens when its caps are met.
pub fn moon_spawn(
    moon: Moon,
    wave: i32,
    count: &dyn Fn(u16) -> usize,
    boss_cap_reached: bool,
    rng: &mut SmallRng,
) -> Option<u16> {
    let table: &[Roll] = match moon {
        Moon::Frost => frost_wave(wave, boss_cap_reached),
        Moon::Pumpkin => pumpkin_wave(wave, boss_cap_reached),
    };
    // The frost moon's flocko is rolled before the wave table, at any wave.
    if moon == Moon::Frost && rng.random_range(0..30) == 0 && count(341) < 4 {
        return Some(341);
    }
    for entry in table {
        if count(entry.npc_type) >= entry.cap {
            continue;
        }
        if entry.one_in > 1 && rng.random_range(0..entry.one_in) != 0 {
            continue;
        }
        return Some(if entry.npc_type == TRASH {
            trash(moon, rng)
        } else {
            entry.npc_type
        });
    }
    None
}

/// The rank and file of each moon, which the tables refer to as one thing.
fn trash(moon: Moon, rng: &mut SmallRng) -> u16 {
    match moon {
        Moon::Pumpkin => rng.random_range(305..315),
        Moon::Frost => rng.random_range(338..341),
    }
}

/// The frost moon's table for a wave.
///
/// Waves fifteen and up are open-ended: they keep the same shape but raise the caps, which is what
/// turns the last third of the event into a wall of Ice Queens rather than a harder version of the
/// same fight.
fn frost_wave(wave: i32, boss_cap: bool) -> &'static [Roll] {
    const W20: [Roll; 3] = [roll(3, 345), roll(2, 346), fallback(344)];
    const W19: [Roll; 4] = [
        capped(10, 345, 4),
        capped(10, 346, 5),
        capped(10, 344, 7),
        fallback(343),
    ];
    const W18: [Roll; 6] = [
        capped(10, 345, 3),
        capped(10, 346, 4),
        capped(10, 344, 6),
        roll(3, 348),
        roll(3, 351),
        fallback(343),
    ];
    const W17: [Roll; 6] = [
        capped(10, 345, 2),
        capped(10, 346, 3),
        capped(10, 344, 5),
        roll(4, 347),
        roll(2, 351),
        fallback(343),
    ];
    const W16: [Roll; 5] = [
        capped(10, 345, 2),
        capped(10, 346, 2),
        capped(10, 344, 4),
        roll(2, 352),
        fallback(343),
    ];
    const W15: [Roll; 5] = [
        capped(10, 345, 1),
        capped(10, 346, 2),
        capped(10, 344, 3),
        roll(3, 347),
        fallback(343),
    ];
    const W14: [Roll; 4] = [
        capped(10, 345, 1),
        capped(10, 346, 1),
        capped(10, 344, 1),
        roll(3, 343),
    ];
    const W13: [Roll; 6] = [
        capped(10, 345, 1),
        capped(10, 346, 1),
        roll(3, 352),
        roll(6, 343),
        roll(3, 342),
        fallback(347),
    ];
    const W12: [Roll; 5] = [
        capped(10, 345, 1),
        capped(10, 344, 1),
        roll(8, 343),
        roll(3, 342),
        fallback(TRASH),
    ];
    const W11: [Roll; 4] = [
        capped(10, 345, 1),
        roll(6, 352),
        roll(2, 342),
        fallback(TRASH),
    ];
    const W10: [Roll; 6] = [
        capped(10, 346, 1),
        capped(10, 344, 2),
        roll(6, 351),
        roll(3, 348),
        roll(3, 347),
        fallback(TRASH),
    ];
    const W9: [Roll; 5] = [
        capped(10, 346, 1),
        capped(10, 344, 1),
        roll(2, 348),
        roll(3, 347),
        fallback(342),
    ];
    const W8: [Roll; 5] = [
        capped(10, 346, 1),
        roll(8, 351),
        roll(3, 348),
        roll(3, 347),
        fallback(350),
    ];
    const W7: [Roll; 4] = [
        capped(10, 346, 1),
        roll(3, 342),
        roll(4, 350),
        fallback(TRASH),
    ];
    const W6: [Roll; 4] = [
        capped(10, 344, 2),
        roll(4, 347),
        roll(2, 348),
        fallback(350),
    ];
    const W5: [Roll; 4] = [
        capped(10, 344, 1),
        roll(4, 350),
        roll(8, 348),
        fallback(TRASH),
    ];
    const W4: [Roll; 4] = [
        capped(10, 344, 1),
        roll(4, 350),
        roll(3, 342),
        fallback(TRASH),
    ];
    const W3: [Roll; 4] = [roll(8, 348), roll(4, 350), roll(3, 342), fallback(TRASH)];
    const W2: [Roll; 2] = [roll(3, 350), fallback(TRASH)];
    const W1: [Roll; 2] = [roll(3, 342), fallback(TRASH)];
    // At the boss cap nothing but the rank and file comes through.
    const NOTHING: [Roll; 0] = [];

    match wave {
        20.. if boss_cap => &NOTHING,
        20.. => &W20,
        19 => &W19,
        18 => &W18,
        17 => &W17,
        16 => &W16,
        15 => &W15,
        14 => &W14,
        13 => &W13,
        12 => &W12,
        11 => &W11,
        10 => &W10,
        9 => &W9,
        8 => &W8,
        7 => &W7,
        6 => &W6,
        5 => &W5,
        4 => &W4,
        3 => &W3,
        2 => &W2,
        _ => &W1,
    }
}

/// The pumpkin moon's table for a wave.
fn pumpkin_wave(wave: i32, boss_cap: bool) -> &'static [Roll] {
    const W20: [Roll; 3] = [capped(2, 327, 2), capped(1, 325, 2), capped(1, 315, 3)];
    const W19: [Roll; 3] = [capped(5, 327, 2), capped(5, 325, 2), capped(1, 315, 5)];
    const W18: [Roll; 4] = [
        capped(7, 327, 2),
        capped(7, 325, 2),
        capped(7, 315, 3),
        fallback(330),
    ];
    const W17: [Roll; 5] = [
        capped(7, 327, 2),
        capped(7, 325, 2),
        capped(7, 315, 2),
        roll(3, 330),
        fallback(329),
    ];
    const W16: [Roll; 5] = [
        capped(10, 327, 2),
        capped(10, 315, 2),
        roll(6, 330),
        roll(3, 329),
        fallback(326),
    ];
    const W15: [Roll; 5] = [
        capped(10, 327, 1),
        capped(7, 325, 2),
        roll(5, 330),
        roll(3, 326),
        fallback(TRASH),
    ];
    const W14: [Roll; 7] = [
        capped(10, 327, 1),
        capped(7, 325, 2),
        capped(10, 315, 1),
        roll(10, 330),
        roll(7, 329),
        roll(3, 326),
        fallback(TRASH),
    ];
    const W13: [Roll; 5] = [
        capped(7, 325, 2),
        capped(10, 315, 2),
        roll(6, 330),
        roll(3, 329),
        fallback(326),
    ];
    const W12: [Roll; 2] = [capped(5, 327, 1), fallback(330)];
    const W11: [Roll; 3] = [capped(7, 325, 2), roll(3, 330), fallback(326)];
    const W10: [Roll; 3] = [capped(10, 327, 1), roll(3, 329), fallback(TRASH)];
    const W9: [Roll; 5] = [
        capped(10, 325, 2),
        roll(8, 330),
        roll(5, 329),
        roll(2, 326),
        fallback(TRASH),
    ];
    const W8: [Roll; 3] = [capped(8, 315, 2), roll(4, 330), fallback(329)];
    const W7: [Roll; 3] = [capped(7, 325, 2), roll(4, 330), fallback(329)];
    const W6: [Roll; 3] = [capped(7, 325, 2), roll(2, 326), fallback(TRASH)];
    const W5: [Roll; 2] = [capped(10, 315, 1), fallback(329)];
    const W4: [Roll; 3] = [capped(8, 330, 1), roll(2, 326), fallback(TRASH)];
    const W3: [Roll; 2] = [roll(3, 329), fallback(326)];
    const W2: [Roll; 2] = [roll(3, 326), fallback(TRASH)];
    const W1: [Roll; 1] = [fallback(TRASH)];
    const NOTHING: [Roll; 0] = [];

    match wave {
        20.. if boss_cap => &NOTHING,
        20.. => &W20,
        19 => &W19,
        18 => &W18,
        17 => &W17,
        16 => &W16,
        15 => &W15,
        14 => &W14,
        13 => &W13,
        12 => &W12,
        11 => &W11,
        10 => &W10,
        9 => &W9,
        8 => &W8,
        7 => &W7,
        6 => &W6,
        5 => &W5,
        4 => &W4,
        3 => &W3,
        2 => &W2,
        _ => &W1,
    }
}

/// What the eclipse puts on the surface.
///
/// Half of it is gated on progression, which is what makes an eclipse before Plantera a different
/// event from one after: no Mothron, no Dr Bones, no Reaper.
pub fn eclipse_spawn(
    downed_plantera: bool,
    downed_all_mechs: bool,
    count: &dyn Fn(u16) -> usize,
    rng: &mut SmallRng,
) -> u16 {
    let one_in = |n: u32, rng: &mut SmallRng| rng.random_range(0..n) == 0;
    if downed_plantera && one_in(80, rng) && count(477) == 0 {
        return 477;
    }
    if one_in(50, rng) && count(251) == 0 {
        return 251;
    }
    if downed_plantera && one_in(5, rng) && count(466) == 0 {
        return 466;
    }
    if downed_plantera && one_in(20, rng) && count(463) == 0 {
        return 463;
    }
    if downed_plantera && one_in(20, rng) && count(467) < 2 {
        return 467;
    }
    if one_in(15, rng) {
        return 159;
    }
    if downed_all_mechs && one_in(13, rng) {
        return 253;
    }
    if one_in(8, rng) {
        return 469;
    }
    if downed_plantera && one_in(7, rng) {
        return 468;
    }
    if downed_plantera && one_in(5, rng) {
        return 460;
    }
    if one_in(4, rng) {
        return 162;
    }
    if one_in(3, rng) {
        return 461;
    }
    if one_in(2, rng) {
        return 462;
    }
    166
}

/// Mothron: the eclipse's boss, and the only reason to fight one after Plantera.
pub const MOTHRON: u16 = 477;

/// A moon as the world keeps it: which one, how far through, and how far into the current wave.
#[derive(Debug, Clone, Copy, Default)]
pub struct MoonState {
    pub moon: Option<Moon>,
    pub wave: i32,
    pub points: f32,
}

impl MoonState {
    pub fn running(&self) -> bool {
        self.moon.is_some()
    }

    /// Raise one, replacing whatever was up. Whichever went up last is the one you are fighting,
    /// and it starts again at wave one however far the other had got.
    pub fn start(&mut self, moon: Moon) {
        self.moon = Some(moon);
        self.wave = 1;
        self.points = 0.0;
    }

    /// Put it away. Dawn does this; there is no other ending.
    pub fn stop(&mut self) -> Option<Moon> {
        self.wave = 0;
        self.points = 0.0;
        self.moon.take()
    }

    /// Count a kill. Returns the wave just reached, if the kill finished one.
    ///
    /// A kill is worth points rather than one, and most of what is on the field during a moon is
    /// worth nothing at all — so what you choose to fight is what decides how far the night gets.
    pub fn note_kill(&mut self, npc_type: u16, expert: bool, master: bool) -> Option<i32> {
        if !self.running() || self.wave >= MOON_LAST_WAVE {
            return None;
        }
        let worth = moon_points(npc_type) as f32 * moon_point_scale(expert, master);
        if worth <= 0.0 {
            return None;
        }
        self.points += worth;
        let quota = MOON_WAVE_POINTS[self.wave.clamp(0, MOON_LAST_WAVE) as usize];
        if quota == 0 || self.points < quota as f32 {
            return None;
        }
        self.points = 0.0;
        self.wave += 1;
        Some(self.wave)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::collections::HashSet;

    fn empty(_: u16) -> usize {
        0
    }

    /// Every wave of both moons sends something into an empty sky.
    #[test]
    fn every_wave_sends_something() {
        for moon in [Moon::Pumpkin, Moon::Frost] {
            for wave in 1..=MOON_LAST_WAVE {
                let mut sent = false;
                for seed in 0..300u64 {
                    let mut rng = SmallRng::seed_from_u64(seed);
                    sent |= moon_spawn(moon, wave, &empty, false, &mut rng).is_some();
                }
                assert!(sent, "{moon:?} wave {wave} sends nothing");
            }
        }
    }

    /// Everything a moon sends is a real type, and one that counts toward its own waves.
    #[test]
    fn moons_only_send_their_own() {
        for moon in [Moon::Pumpkin, Moon::Frost] {
            for wave in 1..=MOON_LAST_WAVE {
                for seed in 0..200u64 {
                    let mut rng = SmallRng::seed_from_u64(seed);
                    let Some(ty) = moon_spawn(moon, wave, &empty, false, &mut rng) else {
                        continue;
                    };
                    assert!(
                        terrustia_proto::npc_data::npc_stats(ty).is_some(),
                        "{moon:?} wave {wave} sent {ty}, which this build has no stats for"
                    );
                    assert!(
                        moon_points(ty) > 0,
                        "{moon:?} wave {wave} sent {ty}, which is worth nothing"
                    );
                    let range = match moon {
                        Moon::Pumpkin => 305..=330,
                        Moon::Frost => 338..=352,
                    };
                    assert!(
                        range.contains(&ty),
                        "{moon:?} sent {ty} from the other roster"
                    );
                }
            }
        }
    }

    /// The bosses only appear once their wave is reached, and the caps really cap them.
    #[test]
    fn the_ice_queen_waits_for_her_wave() {
        let seen = |wave: i32| {
            let mut out = HashSet::new();
            for seed in 0..600u64 {
                let mut rng = SmallRng::seed_from_u64(seed);
                out.extend(moon_spawn(Moon::Frost, wave, &empty, false, &mut rng));
            }
            out
        };
        assert!(!seen(3).contains(&345), "no Ice Queen at wave three");
        assert!(seen(11).contains(&345), "but there is by eleven");

        // ...and not while her cap is met.
        let one_out = |ty: u16| usize::from(ty == 345);
        let mut capped = HashSet::new();
        for seed in 0..600u64 {
            let mut rng = SmallRng::seed_from_u64(seed);
            capped.extend(moon_spawn(Moon::Frost, 11, &one_out, false, &mut rng));
        }
        assert!(!capped.contains(&345), "one at a time at wave eleven");
    }

    /// The Pumpking likewise.
    #[test]
    fn the_pumpking_waits_too() {
        let seen = |wave: i32| {
            let mut out = HashSet::new();
            for seed in 0..600u64 {
                let mut rng = SmallRng::seed_from_u64(seed);
                out.extend(moon_spawn(Moon::Pumpkin, wave, &empty, false, &mut rng));
            }
            out
        };
        assert!(!seen(5).contains(&327), "no Pumpking at wave five");
        assert!(seen(12).contains(&327), "but there is by twelve");
    }

    /// At the boss cap the last wave stops sending bosses — but the frost moon's flocko is
    /// rolled before the wave table and is not one, so it still comes.
    #[test]
    fn the_boss_cap_stops_the_last_wave() {
        for moon in [Moon::Pumpkin, Moon::Frost] {
            for seed in 0..400u64 {
                let mut rng = SmallRng::seed_from_u64(seed);
                let sent = moon_spawn(moon, 20, &empty, true, &mut rng);
                assert!(
                    sent.is_none() || sent == Some(341),
                    "{moon:?} sent {sent:?} at the boss cap"
                );
            }
        }
    }

    /// The quotas rise every wave, so a moon really does get harder rather than longer.
    #[test]
    fn the_quotas_only_rise() {
        for pair in MOON_WAVE_POINTS[1..MOON_LAST_WAVE as usize].windows(2) {
            assert!(pair[1] > pair[0], "{pair:?} does not rise");
        }
        assert_eq!(
            MOON_WAVE_POINTS[MOON_LAST_WAVE as usize], 0,
            "the last wave never completes"
        );
    }

    /// The eclipse's roster grows as the game does.
    #[test]
    fn the_eclipse_grows_with_your_progress() {
        let pool = |plantera: bool, mechs: bool| {
            let mut out = HashSet::new();
            for seed in 0..2000u64 {
                let mut rng = SmallRng::seed_from_u64(seed);
                out.insert(eclipse_spawn(plantera, mechs, &empty, &mut rng));
            }
            out
        };
        let early = pool(false, false);
        assert!(!early.contains(&MOTHRON), "no Mothron before Plantera");
        assert!(!early.contains(&253), "no Reaper before the mechs");

        let late = pool(true, true);
        assert!(late.contains(&MOTHRON), "Mothron after Plantera");
        assert!(late.contains(&253), "and a Reaper once the mechs are down");
        assert!(
            late.len() > early.len(),
            "a wider pool: {late:?} vs {early:?}"
        );

        for ty in late {
            assert!(
                terrustia_proto::npc_data::npc_stats(ty).is_some(),
                "{ty} is not a type this build has"
            );
        }
    }

    /// Raising one moon replaces the other and starts the count again.
    #[test]
    fn one_moon_replaces_the_other() {
        let mut moon = MoonState::default();
        moon.start(Moon::Pumpkin);
        for _ in 0..24 {
            moon.note_kill(305, false, false);
        }
        assert_eq!(moon.moon, Some(Moon::Pumpkin));
        assert!(moon.points > 0.0);

        moon.start(Moon::Frost);
        assert_eq!(moon.moon, Some(Moon::Frost), "the newer one wins");
        assert_eq!(moon.wave, 1, "and starts again");
        assert_eq!(moon.points, 0.0);
    }

    /// A moon advances on points, and only on the points its own roster is worth.
    #[test]
    fn a_moon_advances_on_points() {
        let mut moon = MoonState::default();
        assert!(
            moon.note_kill(327, false, false).is_none(),
            "nothing happens before it starts"
        );

        moon.start(Moon::Pumpkin);
        assert_eq!(moon.wave, 1);
        // Wave one asks for twenty-five points, and a scarecrow is worth one.
        for _ in 0..24 {
            assert_eq!(moon.note_kill(305, false, false), None);
        }
        assert_eq!(
            moon.note_kill(305, false, false),
            Some(2),
            "the twenty-fifth turns it over"
        );
        assert_eq!(moon.points, 0.0, "and the count starts again");
    }

    /// Killing something the moon did not send does not advance it.
    #[test]
    fn a_passing_zombie_does_not_advance_a_moon() {
        let mut moon = MoonState::default();
        moon.start(Moon::Frost);
        for _ in 0..1000 {
            moon.note_kill(3, false, false);
        }
        assert_eq!(moon.wave, 1);
        assert_eq!(moon.points, 0.0);
    }

    /// A Pumpking is worth a third of an early wave by itself.
    #[test]
    fn the_heavies_carry_the_waves() {
        let mut moon = MoonState::default();
        moon.start(Moon::Pumpkin);
        // A hundred and fifty points takes it from wave one straight through four.
        let reached = moon.note_kill(327, false, false);
        assert_eq!(reached, Some(2), "a wave at a time, however big the kill");
        assert_eq!(moon.points, 0.0);
    }

    /// The last wave never turns over, however much you kill in it.
    #[test]
    fn the_last_wave_never_ends() {
        let mut moon = MoonState::default();
        moon.start(Moon::Frost);
        moon.wave = MOON_LAST_WAVE;
        for _ in 0..500 {
            assert_eq!(moon.note_kill(345, true, false), None);
        }
        assert_eq!(moon.wave, MOON_LAST_WAVE);
    }

    /// Expert doubles what a kill is worth, master two and a half times it, so a moon runs
    /// further in one night — and a Journey world's continuous `DifficultySlider` still only ever
    /// reaches this as one of these two booleans (`is_expert()`/`is_master()`), never a raw
    /// `game_mode`, which is exactly why this function takes bools and not a difficulty number.
    #[test]
    fn expert_doubles_the_score() {
        assert_eq!(moon_point_scale(false, false), 1.0, "classic");
        assert_eq!(moon_point_scale(true, false), 2.0, "expert");
        assert_eq!(moon_point_scale(true, true), 2.5, "master");
        assert_eq!(
            moon_point_scale(false, true),
            2.5,
            "master alone (unreachable via is_expert/is_master, but the function's own contract)"
        );
    }
}
