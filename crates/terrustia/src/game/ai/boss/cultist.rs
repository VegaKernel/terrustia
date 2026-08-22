//! The Lunatic Cultist: style 84.
//!
//! Its attacks come off a fixed script rather than a die roll, and every other entry in that script
//! is "move" — which is why it spends so much of the fight drifting into a new position above you.
//! The sequence never varies, so the fight is memorisable, and that is deliberate.
//!
//! What makes it *this* fight is the ritual. Partway through the script it makes four clones of
//! itself and they all move and cast in lockstep. Only the real one advances the fight when hit;
//! hitting a clone kills that clone and leaves the rest of them casting. Guessing wrong is not free
//! — ten of the lights they have put out survive it, or three in expert.
//!
//! A clone is the same routine reading its owner's state rather than deciding anything of its own,
//! which is exactly why they are indistinguishable while the ritual runs.

use rand::rngs::SmallRng;
use terrustia_proto::npc_params::{
    CULTIST_ARRIVAL, CULTIST_CLONE, CULTIST_CLONES, CULTIST_FIRE, CULTIST_FIRE_COUNT,
    CULTIST_FIRE_COUNT_EXPERT, CULTIST_FIRE_DAMAGE, CULTIST_FIRE_EVERY, CULTIST_FIRE_EVERY_EXPERT,
    CULTIST_HALF_DEFENSE, CULTIST_ICE, CULTIST_ICE_DAMAGE, CULTIST_ICE_EVERY,
    CULTIST_ICE_EVERY_EXPERT, CULTIST_LIGHTNING, CULTIST_LIGHTNING_DAMAGE, CULTIST_LIGHTNING_EVERY,
    CULTIST_LIGHTNING_EVERY_EXPERT, CULTIST_MOVE_STEP, CULTIST_ORBIT, CULTIST_ORBIT_SPREAD,
    CULTIST_PAUSE, CULTIST_RITUAL_TICKS, CULTIST_RITUAL_WINDOW, CULTIST_SCRIPT,
};

use super::skeletron::Parent;
use crate::game::ai::{Shot, World};
use crate::game::npc::{Npc, TileView};
use crate::game::npc_ai::Spawn;

/// The states, as `ai[0]` numbers them.
mod state {
    pub const ARRIVING: f32 = -1.0;
    pub const DECIDING: f32 = 0.0;
    pub const MOVING: f32 = 1.0;
    pub const FIREBALLS: f32 = 2.0;
    pub const ICE: f32 = 3.0;
    pub const LIGHTNING: f32 = 4.0;
    pub const RITUAL: f32 = 5.0;
}

/// What it did this tick.
#[derive(Debug, Default)]
pub struct CultistOutcome {
    pub shots: Vec<Shot>,
    pub spawn: Vec<Spawn>,
    /// Set when this one has outlived whatever it was a copy of.
    pub spent: bool,
    /// Where it wants to move to, worked out once at the start of a reposition.
    pub move_to: Option<(f32, f32)>,
}

/// Style 84.
///
/// `owner` is `None` for the real cultist and the state of the real one for a clone. `clones` is
/// how many copies are already out.
pub fn cultist(
    npc: &mut Npc,
    world: &World<'_, impl TileView>,
    owner: Option<Parent>,
    clones: usize,
    _rng: &mut SmallRng,
) -> CultistOutcome {
    let mut out = CultistOutcome::default();
    npc.dirty = true;
    let real = npc.npc_type != CULTIST_CLONE;
    let expert = world.conditions.expert;
    let health = npc.life as f32 / npc.life_max.max(1) as f32;
    let wounded = health <= 0.5;
    if wounded {
        npc.defense = (npc.stats.defense as f32 * CULTIST_HALF_DEFENSE) as i32;
    }

    // A clone has no mind of its own: it copies the real one's state exactly, which is what makes
    // the two indistinguishable.
    if !real {
        let Some(owner) = owner else {
            out.spent = true;
            return out;
        };
        npc.ai[0] = owner.state;
        // A clone cannot be hurt into advancing the fight; it simply dies.
        if npc.ai[0] != state::RITUAL {
            npc.invulnerable = true;
        }
    }

    // The arrival: seven seconds of fading in, during which nothing touches it.
    if npc.local_ai[0] == 0.0 {
        npc.local_ai[0] = 1.0;
        npc.alpha = 255;
        npc.ai[0] = state::ARRIVING;
    }

    let Some(target) = world.target.filter(|t| t.alive) else {
        return out;
    };
    let (cx, cy) = npc.center();

    match npc.ai[0] {
        state::ARRIVING => {
            npc.invulnerable = true;
            npc.alpha = (npc.alpha - 5).max(0);
            npc.ai[1] += 1.0;
            if npc.ai[1] >= CULTIST_ARRIVAL {
                npc.ai[0] = state::DECIDING;
                npc.ai[1] = 0.0;
                npc.invulnerable = false;
            } else if npc.ai[1] > 360.0 {
                npc.velocity.0 *= 0.95;
                npc.velocity.1 *= 0.95;
            } else if npc.ai[1] > 300.0 {
                npc.velocity = (0.0, -1.0);
            }
        }

        state::DECIDING => {
            npc.direction = (target.center.0 - cx).signum() as i8;
            npc.sprite_direction = npc.direction;
            npc.ai[1] += 1.0;
            if npc.ai[1] < CULTIST_PAUSE || !real {
                return out;
            }
            // The script, indexed by how many attacks it has made. Running off the end restarts it.
            let step = npc.ai[3] as usize;
            let what = CULTIST_SCRIPT.get(step).copied().unwrap_or(0);
            if step + 1 >= CULTIST_SCRIPT.len() {
                npc.ai[3] = -1.0;
            }
            npc.ai[1] = 0.0;
            npc.ai[0] = match what {
                1 => state::ICE,
                2 => state::FIREBALLS,
                3 => state::LIGHTNING,
                4 => state::RITUAL,
                _ => {
                    // Move. The destination is an ellipse above the player, and the whole group
                    // shares it out so they fan rather than stack.
                    let station = orbit_slot(target.center, 0, clones + 1);
                    let gap = (station.0 - cx, station.1 - cy);
                    let steps = (gap.0.hypot(gap.1) / CULTIST_MOVE_STEP).ceil().max(1.0);
                    npc.velocity = (gap.0 / steps, gap.1 / steps);
                    npc.ai[1] = steps * 2.0;
                    out.move_to = Some(station);
                    state::MOVING
                }
            };
        }

        state::MOVING => {
            // It travels in steps rather than smoothly, holding still every other tick.
            if (npc.ai[1] as i32) % 2 != 0 && npc.ai[1] != 1.0 {
                npc.position.0 -= npc.velocity.0;
                npc.position.1 -= npc.velocity.1;
            }
            npc.ai[1] -= 1.0;
            if npc.ai[1] <= 0.0 {
                npc.ai[0] = state::DECIDING;
                npc.ai[1] = 0.0;
                npc.velocity = (0.0, 0.0);
                if real {
                    npc.ai[3] += 1.0;
                }
            }
        }

        state::FIREBALLS => {
            let every = if expert {
                CULTIST_FIRE_EVERY_EXPERT
            } else {
                CULTIST_FIRE_EVERY
            };
            let count = if expert {
                CULTIST_FIRE_COUNT_EXPERT
            } else {
                CULTIST_FIRE_COUNT
            };
            npc.ai[1] += 1.0;
            if npc.ai[1] >= 4.0 && (npc.ai[1] as i32 - 4) % every as i32 == 0 {
                out.shots.push(aimed(
                    npc,
                    target.center,
                    CULTIST_FIRE,
                    CULTIST_FIRE_DAMAGE,
                    9.0,
                ));
            }
            if npc.ai[1] >= 4.0 + every * count as f32 {
                finish(npc, real);
            }
        }

        state::ICE => {
            let every = if expert {
                CULTIST_ICE_EVERY_EXPERT
            } else {
                CULTIST_ICE_EVERY
            };
            npc.ai[1] += 1.0;
            if npc.ai[1] as i32 == every as i32 {
                out.shots.push(aimed(
                    npc,
                    target.center,
                    CULTIST_ICE,
                    CULTIST_ICE_DAMAGE,
                    6.0,
                ));
            }
            if npc.ai[1] >= every + 20.0 {
                finish(npc, real);
            }
        }

        state::LIGHTNING => {
            let every = if expert {
                CULTIST_LIGHTNING_EVERY_EXPERT
            } else {
                CULTIST_LIGHTNING_EVERY
            };
            npc.ai[1] += 1.0;
            if npc.ai[1] as i32 == every as i32 / 2 {
                out.shots.push(aimed(
                    npc,
                    target.center,
                    CULTIST_LIGHTNING,
                    CULTIST_LIGHTNING_DAMAGE,
                    7.0,
                ));
            }
            if npc.ai[1] >= every {
                finish(npc, real);
            }
        }

        _ => {
            // The ritual. It makes its clones once and then they all cast together.
            if real && npc.ai[1] == 0.0 && clones == 0 {
                for _ in 0..CULTIST_CLONES {
                    out.spawn.push(Spawn {
                        npc_type: CULTIST_CLONE,
                        position: npc.center(),
                        velocity: (0.0, 0.0),
                        parent: Some(Spawn::OWN_PARENT),
                    });
                }
            }
            npc.ai[1] += 1.0;
            if npc.ai[1] >= CULTIST_RITUAL_TICKS {
                finish(npc, real);
            }
        }
    }
    out
}

/// Whether hitting this one right now is the guess that advances the fight.
///
/// Only during the ritual, and only in the window after the clones have settled — a hit before
/// that is not a guess, it is a stray shot from the previous attack.
pub fn is_a_guess(npc: &Npc) -> bool {
    npc.ai[0] == state::RITUAL
        && npc.ai[1] >= CULTIST_RITUAL_WINDOW.0
        && npc.ai[1] < CULTIST_RITUAL_WINDOW.1
}

/// One of the positions on the ellipse above the player, shared out between the group.
fn orbit_slot(player: (f32, f32), index: usize, of: usize) -> (f32, f32) {
    let even = of.is_multiple_of(2);
    let step = (index + usize::from(even)).div_ceil(2) as f32;
    let mut angle = step * std::f32::consts::TAU * CULTIST_ORBIT_SPREAD / of as f32;
    if index % 2 == 1 {
        angle = -angle;
    }
    if of == 1 {
        angle = 0.0;
    }
    // Straight up, rotated, then squashed into the ellipse.
    let (sin, cos) = angle.sin_cos();
    (
        player.0 + sin * CULTIST_ORBIT.0,
        player.1 - cos * CULTIST_ORBIT.1,
    )
}

/// A shot aimed at the player, from the caster's middle.
fn aimed(npc: &Npc, player: (f32, f32), projectile: u16, damage: i32, speed: f32) -> Shot {
    let (cx, cy) = npc.center();
    let aim = (player.0 - cx, player.1 - cy);
    let length = aim.0.hypot(aim.1).max(f32::MIN_POSITIVE);
    Shot {
        projectile,
        damage,
        position: (cx, cy),
        velocity: (aim.0 / length * speed, aim.1 / length * speed),
        time_left: 600,
    }
}

/// End an attack and step the script on, but only for the real one — a clone follows.
fn finish(npc: &mut Npc, real: bool) {
    npc.ai[0] = state::DECIDING;
    npc.ai[1] = 0.0;
    if real {
        npc.ai[3] += 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use rand::SeedableRng;
    use std::collections::HashMap;
    use terrustia_proto::npc_params::CULTIST;
    use terrustia_proto::tile::Tile;

    struct Sky(HashMap<(i32, i32), Tile>);

    impl TileView for Sky {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn world<'a>(tiles: &'a Sky, target: Option<(f32, f32)>) -> World<'a, Sky> {
        crate::game::ai::calm(
            tiles,
            target.map(|center| Target {
                slot: 0,
                center,
                velocity: (0.0, 0.0),
                alive: true,
            }),
        )
    }

    fn owner_in(state: f32) -> Parent {
        Parent {
            position: (0.0, 0.0),
            size: (40.0, 60.0),
            rotation: 0.0,
            scale: 1.0,
            velocity: (0.0, 0.0),
            direction: 1,
            sprite_direction: 1,
            time_left: 3600,
            state,
            health: 1.0,
        }
    }

    fn caster(npc_type: u16) -> Npc {
        let mut npc = Npc::new(npc_type, (0.0, 0.0), 1).expect("a cultist");
        // Past the arrival, so the tests are about the fight rather than the entrance.
        npc.local_ai[0] = 1.0;
        npc.ai[0] = state::DECIDING;
        npc
    }

    /// It cannot be touched while it is arriving.
    #[test]
    fn it_arrives_untouchable() {
        let tiles = Sky(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(84);
        let mut c = Npc::new(CULTIST, (0.0, 0.0), 1).unwrap();
        let w = world(&tiles, Some((300.0, 0.0)));

        cultist(&mut c, &w, None, 0, &mut rng);
        assert_eq!(c.ai[0], state::ARRIVING);
        assert!(c.invulnerable, "nothing lands on it yet");

        for _ in 0..(CULTIST_ARRIVAL as i32 + 2) {
            cultist(&mut c, &w, None, 0, &mut rng);
        }
        assert_eq!(c.ai[0], state::DECIDING, "then it starts");
        assert!(!c.invulnerable);
    }

    /// The script is fixed, so two fights run in the same order.
    #[test]
    fn its_attacks_come_in_a_fixed_order() {
        let tiles = Sky(HashMap::new());
        let w = world(&tiles, Some((300.0, 200.0)));
        let sequence = |seed: u64| {
            let mut rng = SmallRng::seed_from_u64(seed);
            let mut c = caster(CULTIST);
            let mut seen = Vec::new();
            let mut was = c.ai[0];
            for _ in 0..4000 {
                cultist(&mut c, &w, None, 0, &mut rng);
                if c.ai[0] != was {
                    if c.ai[0] != state::DECIDING {
                        seen.push(c.ai[0]);
                    }
                    was = c.ai[0];
                }
                if seen.len() >= 8 {
                    break;
                }
            }
            seen
        };
        assert_eq!(
            sequence(1),
            sequence(999),
            "the script should not depend on the dice"
        );
        assert!(
            sequence(1).contains(&state::ICE),
            "and should include its attacks: {:?}",
            sequence(1)
        );
    }

    /// A clone copies the real one's state rather than deciding for itself.
    #[test]
    fn a_clone_mirrors_the_real_one() {
        let tiles = Sky(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(2);
        let w = world(&tiles, Some((300.0, 0.0)));
        let mut clone = caster(CULTIST_CLONE);

        cultist(&mut clone, &w, Some(owner_in(state::ICE)), 4, &mut rng);
        assert_eq!(clone.ai[0], state::ICE, "it does what the real one does");

        cultist(
            &mut clone,
            &w,
            Some(owner_in(state::FIREBALLS)),
            4,
            &mut rng,
        );
        assert_eq!(clone.ai[0], state::FIREBALLS);
    }

    /// A clone with no original does not survive.
    #[test]
    fn a_clone_without_an_original_is_gone() {
        let tiles = Sky(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(3);
        let w = world(&tiles, Some((300.0, 0.0)));
        let mut clone = caster(CULTIST_CLONE);
        assert!(cultist(&mut clone, &w, None, 0, &mut rng).spent);
    }

    /// The ritual makes its clones, once.
    #[test]
    fn the_ritual_makes_four_clones() {
        let tiles = Sky(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(5);
        let w = world(&tiles, Some((300.0, 0.0)));
        let mut c = caster(CULTIST);
        c.ai[0] = state::RITUAL;

        let out = cultist(&mut c, &w, None, 0, &mut rng);
        assert_eq!(out.spawn.len(), CULTIST_CLONES);
        assert!(out.spawn.iter().all(|s| s.npc_type == CULTIST_CLONE));
        // With clones already out it makes no more.
        assert!(cultist(&mut c, &w, None, 4, &mut rng).spawn.is_empty());
    }

    /// Hitting one only counts as a guess inside the ritual's window.
    #[test]
    fn only_a_hit_during_the_ritual_is_a_guess() {
        let mut c = Npc::new(CULTIST, (0.0, 0.0), 1).unwrap();
        c.ai[0] = state::ICE;
        assert!(!is_a_guess(&c), "not while it is casting");

        c.ai[0] = state::RITUAL;
        c.ai[1] = 10.0;
        assert!(!is_a_guess(&c), "nor before the clones have settled");

        c.ai[1] = CULTIST_RITUAL_WINDOW.0 + 10.0;
        assert!(is_a_guess(&c), "but inside the window it is");

        c.ai[1] = CULTIST_RITUAL_WINDOW.1 + 10.0;
        assert!(!is_a_guess(&c), "and not after it has passed");
    }

    /// The group fans out around the player rather than stacking on one spot.
    #[test]
    fn the_group_shares_out_its_positions() {
        let player = (0.0, 0.0);
        let slots: Vec<(f32, f32)> = (0..5).map(|i| orbit_slot(player, i, 5)).collect();
        for (i, a) in slots.iter().enumerate() {
            for b in &slots[i + 1..] {
                assert!(
                    (a.0 - b.0).abs() > 1.0 || (a.1 - b.1).abs() > 1.0,
                    "two of them wanted the same place: {a:?} and {b:?}"
                );
            }
        }
        // All above the player, which is where the ellipse sits.
        assert!(slots.iter().all(|s| s.1 < player.1), "{slots:?}");
    }

    /// Below half health it sheds a third of its armour.
    #[test]
    fn a_hurt_cultist_is_softer() {
        let tiles = Sky(HashMap::new());
        let mut rng = SmallRng::seed_from_u64(7);
        let w = world(&tiles, Some((300.0, 0.0)));
        let mut c = caster(CULTIST);
        c.life = c.life_max / 4;
        cultist(&mut c, &w, None, 0, &mut rng);
        assert!(
            c.defense < c.stats.defense,
            "{} should be under {}",
            c.defense,
            c.stats.defense
        );
    }
}
