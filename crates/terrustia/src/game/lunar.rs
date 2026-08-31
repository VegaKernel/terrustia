//! The Lunar Apocalypse: four pillars, and what comes after them.
//!
//! Killing the Lunatic Cultist tears four holes in the world, a fifth of it apart, and a pillar
//! comes out of each. A pillar cannot be hurt at all while its shield holds, and the shield is not
//! a health bar — it is a count of how many of its own minions are still alive to be killed. A
//! hundred of them per pillar, half that once you have beaten the Moon Lord before.
//!
//! That is the whole design of the event: you do not fight the pillars, you clear the ground
//! around them, and the pillar is what is left. When the last one falls the sky counts down a
//! minute and the Moon Lord arrives on whoever is nearest the middle of the world.

use rand::{Rng, rngs::SmallRng};

/// The four pillars, in the game's own ids, and the shield each one carries.
///
/// One source, in the data crate: these were declared twice, here and in `npc_params`, with the
/// copy here the only one anything read.
pub use terrustia_proto::npc_params::{
    TOWER_NEBULA as NEBULA, TOWER_SHIELD as SHIELD_STRENGTH, TOWER_SOLAR as SOLAR,
    TOWER_STARDUST as STARDUST, TOWER_VORTEX as VORTEX,
};

pub const PILLARS: [u16; 4] = [SOLAR, VORTEX, NEBULA, STARDUST];

/// The Moon Lord's core, which is what actually arrives.
pub const MOON_LORD: u16 = 398;

/// How long the sky takes after the last pillar falls.
pub const MOON_LORD_COUNTDOWN: i32 = 3600;

/// Which pillar an NPC belongs to, if any.
///
/// A minion only counts toward the pillar it came from, so clearing solar fragments does nothing
/// for the vortex tower standing next to it.
pub fn belongs_to(npc_type: u16) -> Option<u16> {
    match npc_type {
        412..=419 | 518 => Some(SOLAR),
        425..=427 | 429 => Some(VORTEX),
        420 | 421 | 423 | 424 => Some(NEBULA),
        402 | 405 | 407 | 409 | 411 => Some(STARDUST),
        _ => None,
    }
}

/// The event as the world keeps it.
#[derive(Debug, Clone, Copy, Default)]
pub struct LunarState {
    /// Whether the pillars are up.
    pub up: bool,
    /// Each pillar's remaining shield, in the order of [`PILLARS`].
    pub shields: [i32; 4],
    /// Ticks until the Moon Lord arrives. Zero means nothing is coming.
    pub countdown: i32,
}

impl LunarState {
    /// Tear the sky open. Returns where the four pillars want to stand, in tiles.
    ///
    /// They are placed a fifth of the world apart with a hundred tiles of slop, and which pillar
    /// goes where is drawn rather than fixed — so no two runs put the solar tower in the same
    /// place, and you cannot learn the route.
    pub fn trigger(
        &mut self,
        world_width: i32,
        surface: i32,
        downed_moon_lord: bool,
        rng: &mut SmallRng,
    ) -> Vec<(u16, i32, i32)> {
        let strength = if downed_moon_lord {
            SHIELD_STRENGTH / 2
        } else {
            SHIELD_STRENGTH
        };
        self.up = true;
        self.shields = [strength; 4];
        self.countdown = 0;

        let mut draw: Vec<u16> = PILLARS.to_vec();
        let mut order = Vec::with_capacity(4);
        while !draw.is_empty() {
            order.push(draw.remove(rng.random_range(0..draw.len())));
        }

        let step = world_width / 5;
        order
            .into_iter()
            .enumerate()
            .map(|(i, npc_type)| {
                let x = step * (1 + i as i32) + rng.random_range(-100..=100);
                (npc_type, x, surface - 40)
            })
            .collect()
    }

    /// Count a kill against whichever pillar's escort it belonged to.
    ///
    /// Returns the pillar whose shield just fell, if this was the kill that dropped one.
    pub fn note_kill(&mut self, npc_type: u16) -> Option<u16> {
        if !self.up {
            return None;
        }
        let pillar = belongs_to(npc_type)?;
        let slot = PILLARS.iter().position(|p| *p == pillar)?;
        if self.shields[slot] <= 0 {
            return None;
        }
        self.shields[slot] -= 1;
        (self.shields[slot] == 0).then_some(pillar)
    }

    /// What a pillar's shield reads as right now.
    pub fn shield_of(&self, npc_type: u16) -> i32 {
        PILLARS
            .iter()
            .position(|p| *p == npc_type)
            .map_or(0, |slot| self.shields[slot])
    }

    /// One tick. `pillars_alive` is how many of the four are still standing, and `moon_lord` is
    /// whether the thing they were holding back is already here.
    ///
    /// Returns `true` on the tick the Moon Lord should arrive.
    pub fn tick(&mut self, pillars_alive: usize, moon_lord_here: bool) -> bool {
        if self.up && pillars_alive == 0 && !moon_lord_here {
            // The last pillar has fallen: the sky starts counting.
            self.up = false;
            self.countdown = MOON_LORD_COUNTDOWN;
        }
        if self.countdown > 0 {
            self.countdown -= 1;
            return self.countdown == 0;
        }
        false
    }

    /// Give up on the whole thing — the Moon Lord is down, or was beaten to it.
    pub fn stop(&mut self) {
        self.up = false;
        self.countdown = 0;
        self.shields = [0; 4];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use std::collections::HashSet;

    /// It raises all four, spread across the world, in an order that is not the same twice.
    #[test]
    fn it_raises_four_pillars_across_the_world() {
        let mut orders = HashSet::new();
        for seed in 0..40u64 {
            let mut rng = SmallRng::seed_from_u64(seed);
            let mut lunar = LunarState::default();
            let raised = lunar.trigger(8400, 400, false, &mut rng);
            assert_eq!(raised.len(), 4);

            let kinds: Vec<u16> = raised.iter().map(|(ty, _, _)| *ty).collect();
            let unique: HashSet<u16> = kinds.iter().copied().collect();
            assert_eq!(unique.len(), 4, "one of each: {kinds:?}");
            assert!(unique.iter().all(|ty| PILLARS.contains(ty)));
            orders.insert(kinds);

            // A fifth of the world apart, give or take the slop.
            let mut xs: Vec<i32> = raised.iter().map(|(_, x, _)| *x).collect();
            xs.sort_unstable();
            for pair in xs.windows(2) {
                let gap = pair[1] - pair[0];
                assert!(
                    (8400 / 5 - 200..=8400 / 5 + 200).contains(&gap),
                    "pillars {gap} apart"
                );
            }
        }
        assert!(orders.len() > 1, "the order should not be fixed");
    }

    /// The shield is a body count, and only its own escort counts toward it.
    #[test]
    fn a_shield_falls_only_to_its_own_escort() {
        let mut rng = SmallRng::seed_from_u64(1);
        let mut lunar = LunarState::default();
        lunar.trigger(8400, 400, false, &mut rng);
        assert_eq!(lunar.shield_of(SOLAR), SHIELD_STRENGTH);

        // Vortex minions do nothing for the solar tower.
        for _ in 0..200 {
            lunar.note_kill(425);
        }
        assert_eq!(lunar.shield_of(SOLAR), SHIELD_STRENGTH);
        assert_eq!(lunar.shield_of(VORTEX), 0, "but they clear their own");

        let mut fell = None;
        for _ in 0..SHIELD_STRENGTH {
            if let Some(pillar) = lunar.note_kill(412) {
                fell = Some(pillar);
            }
        }
        assert_eq!(fell, Some(SOLAR));
        assert_eq!(lunar.shield_of(SOLAR), 0);
    }

    /// Beating the Moon Lord once halves every shield after.
    #[test]
    fn a_second_run_is_half_the_work() {
        let mut rng = SmallRng::seed_from_u64(2);
        let mut lunar = LunarState::default();
        lunar.trigger(8400, 400, true, &mut rng);
        assert_eq!(lunar.shield_of(SOLAR), SHIELD_STRENGTH / 2);
    }

    /// Nothing counts before the event starts, or after it ends.
    #[test]
    fn kills_outside_the_event_do_nothing() {
        let mut lunar = LunarState::default();
        assert_eq!(lunar.note_kill(412), None);

        let mut rng = SmallRng::seed_from_u64(3);
        lunar.trigger(8400, 400, false, &mut rng);
        lunar.stop();
        assert_eq!(lunar.note_kill(412), None);
    }

    /// The last pillar starts a minute's countdown, and then he arrives.
    #[test]
    fn the_last_pillar_calls_him_down() {
        let mut rng = SmallRng::seed_from_u64(4);
        let mut lunar = LunarState::default();
        lunar.trigger(8400, 400, false, &mut rng);

        // While any pillar stands, nothing counts down.
        for _ in 0..1000 {
            assert!(!lunar.tick(2, false));
            assert_eq!(lunar.countdown, 0);
        }

        let mut arrived = None;
        for at in 1..(MOON_LORD_COUNTDOWN + 100) {
            if lunar.tick(0, false) {
                arrived = Some(at);
                break;
            }
        }
        assert_eq!(arrived, Some(MOON_LORD_COUNTDOWN));
        assert!(!lunar.up, "and the pillars are behind you");
    }

    /// He does not arrive twice.
    #[test]
    fn he_does_not_arrive_twice() {
        let mut rng = SmallRng::seed_from_u64(5);
        let mut lunar = LunarState::default();
        lunar.trigger(8400, 400, false, &mut rng);
        for _ in 0..(MOON_LORD_COUNTDOWN * 2) {
            lunar.tick(0, true);
        }
        assert_eq!(lunar.countdown, 0, "with him already here, nothing counts");
    }

    /// Every minion the table names is a type this build has, and belongs to exactly one pillar.
    #[test]
    fn the_escorts_are_real_and_unambiguous() {
        let mut counted = 0;
        for npc_type in 0..terrustia_proto::npc_data::NPC_COUNT {
            let Some(pillar) = belongs_to(npc_type) else {
                continue;
            };
            counted += 1;
            assert!(PILLARS.contains(&pillar));
            assert!(
                terrustia_proto::npc_data::npc_stats(npc_type).is_some(),
                "{npc_type} belongs to a pillar but this build has no stats for it"
            );
        }
        assert!(counted >= 20, "only {counted} escorts found");
    }
}
