//! Style 55 — the Creepers that orbit the Brain of Cthulhu.
//!
//! Ported from the `aiStyle == 55` block. A creeper is tethered to the Brain: while it is more
//! than 90 pixels away it steers back with a heavy smoothing filter, and once inside that radius
//! it speeds up and occasionally breaks off to charge the player.

use super::World;
use crate::game::npc::{Npc, TileView};

/// How far a creeper may drift from the Brain before it is pulled back.
pub const TETHER_RANGE: f32 = 90.0;

/// A charging creeper more than this far from the Brain gives up and returns to orbit
/// (`AI_055`: `num885 > 700f`).
pub const CHARGE_RETURN_RANGE: f32 = 700.0;

/// Speed of both the return and the charge.
pub const SPEED: f32 = 8.0;

/// Chance per tick of breaking off to charge, as a one-in-N roll.
pub const CHARGE_CHANCE: u32 = 200;

/// Expert mode doubles how often they charge.
pub const CHARGE_CHANCE_EXPERT: u32 = 100;

/// What a creeper wants to do this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The Brain is gone, so the creeper should be removed.
    BrainGone,
    Alive,
}

/// Drive one creeper.
///
/// `brain` is the Brain's centre, or `None` when it is dead. `charge_roll` is the caller's draw
/// (one in 200, or one in 100 as well in expert) so the behaviour stays deterministic under test.
pub fn update<T: TileView>(
    npc: &mut Npc,
    world: &World<'_, T>,
    brain: Option<(f32, f32)>,
    charge_roll: bool,
) -> Outcome {
    let target = world.target;
    let Some(brain_center) = brain else {
        return Outcome::BrainGone;
    };

    // ai[0] marks a creeper that has committed to a charge; it stops orbiting until it resets.
    // Vanilla `AI_055` resets it (back to orbit) once the creeper strays more than 700px from the
    // Brain (`NPC.cs:32968-32972`) - without this the creeper flew straight forever after one dart
    // and the swarm scattered off and never re-formed. Its charge velocity carries it there under
    // normal physics.
    if npc.ai[0] != 0.0 {
        // `NPC.cs:32950-32965`: in expert a charge is not a straight line. It keeps bending toward
        // the player the whole way in, hard enough in a get-good world to be nearly unavoidable.
        if world.conditions.expert
            && let Some(t) = target
        {
            let (cx, cy) = npc.center();
            let (dx, dy) = (t.center.0 - cx, t.center.1 - cy);
            let d = (dx * dx + dy * dy).sqrt();
            if d > 0.0 {
                let (blend, pull) = if world.conditions.get_good_world {
                    (50.0, 12.0)
                } else {
                    (100.0, 9.0)
                };
                npc.velocity = (
                    (npc.velocity.0 * (blend - 1.0) + dx / d * pull) / blend,
                    (npc.velocity.1 * (blend - 1.0) + dy / d * pull) / blend,
                );
                npc.dirty = true;
            }
        }

        let (cx, cy) = npc.center();
        let (dx, dy) = (brain_center.0 - cx, brain_center.1 - cy);
        if (dx * dx + dy * dy).sqrt() > CHARGE_RETURN_RANGE {
            npc.ai[0] = 0.0;
            npc.dirty = true;
        } else if world.was_hurt {
            // `NPC.cs:32974-32990`. A creeper that takes a hit while charging goes back to orbit.
            // The game counts five hits first for a type that cannot be knocked back at all; the
            // Creeper's `knockBackResist` is 0.8, so it takes the immediate arm.
            if npc.stats.knockback_resist == 0.0 {
                npc.ai[1] += 1.0;
                if npc.ai[1] > 5.0 {
                    npc.ai[0] = 0.0;
                }
            } else {
                npc.ai[0] = 0.0;
            }
            npc.dirty = true;
        }
        return Outcome::Alive;
    }

    npc.ai[1] = 0.0;
    let (cx, cy) = npc.center();
    let (dx, dy) = (brain_center.0 - cx, brain_center.1 - cy);
    let distance = (dx * dx + dy * dy).sqrt();

    if distance > TETHER_RANGE {
        // Steer home, blending fifteen parts old velocity to one part new.
        let scale = SPEED / distance;
        npc.velocity.0 = (npc.velocity.0 * 15.0 + dx * scale) / 16.0;
        npc.velocity.1 = (npc.velocity.1 * 15.0 + dy * scale) / 16.0;
        npc.dirty = true;
        return Outcome::Alive;
    }

    // Inside the ring: wind up until it is moving at full tilt.
    if npc.velocity.0.abs() + npc.velocity.1.abs() < SPEED {
        npc.velocity.0 *= 1.05;
        npc.velocity.1 *= 1.05;
    }

    if charge_roll && let Some(t) = target {
        let (ddx, ddy) = (t.center.0 - cx, t.center.1 - cy);
        let d = (ddx * ddx + ddy * ddy).sqrt().max(0.001);
        let scale = SPEED / d;
        npc.velocity = (ddx * scale, ddy * scale);
        npc.ai[0] = 1.0;
        npc.dirty = true;
    }
    Outcome::Alive
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::npc_ai::Target;
    use std::collections::HashMap;
    use terrustia_proto::tile::Tile;

    #[derive(Default)]
    struct Cave(HashMap<(i32, i32), Tile>);

    impl TileView for Cave {
        fn tile(&self, x: i32, y: i32) -> Tile {
            self.0.get(&(x, y)).copied().unwrap_or(Tile::AIR)
        }
    }

    fn creeper(at: (f32, f32)) -> Npc {
        Npc::new(267, at, 1).expect("creeper")
    }

    fn player(x: f32, y: f32) -> Target {
        Target {
            slot: 0,
            center: (x, y),
            velocity: (0.0, 0.0),
            alive: true,
        }
    }

    fn world<'a>(tiles: &'a Cave, target: Option<Target>) -> World<'a, Cave> {
        crate::game::ai::calm(tiles, target)
    }

    #[test]
    fn a_creeper_without_a_brain_asks_to_be_removed() {
        let tiles = Cave::default();
        let mut c = creeper((0.0, 0.0));
        assert_eq!(
            update(&mut c, &world(&tiles, None), None, false),
            Outcome::BrainGone
        );
    }

    #[test]
    fn a_charging_creeper_returns_to_orbit_once_it_strays_from_the_brain() {
        let tiles = Cave::default();
        let brain = (1000.0, 1000.0);
        // Committed to a charge (ai[0] set) but now well over 700px from the Brain.
        let mut strayed = creeper((0.0, 0.0));
        strayed.ai[0] = 1.0;
        update(&mut strayed, &world(&tiles, None), Some(brain), false);
        assert_eq!(
            strayed.ai[0], 0.0,
            "a strayed creeper drops its charge and re-orbits instead of flying off forever"
        );

        // Still charging while close to the Brain.
        let mut close = creeper((985.0, 1000.0));
        close.ai[0] = 1.0;
        update(&mut close, &world(&tiles, None), Some(brain), false);
        assert_eq!(close.ai[0], 1.0, "a creeper near the Brain stays committed");
    }

    #[test]
    fn a_distant_creeper_steers_home_gradually() {
        let tiles = Cave::default();
        // Well outside the tether, directly left of the brain.
        let mut c = creeper((0.0, 0.0));
        let (cx, cy) = c.center();
        let brain = (cx + 500.0, cy);

        update(&mut c, &world(&tiles, None), Some(brain), false);
        // The smoothing means one tick moves it only a sixteenth of the way to full speed.
        assert!(
            c.velocity.0 > 0.0 && c.velocity.0 < 1.0,
            "got {}",
            c.velocity.0
        );

        for _ in 0..200 {
            update(&mut c, &world(&tiles, None), Some(brain), false);
        }
        assert!(
            (c.velocity.0 - SPEED).abs() < 0.5,
            "should converge on the tether speed, got {}",
            c.velocity.0
        );
    }

    #[test]
    fn a_creeper_inside_the_ring_winds_up_rather_than_steering() {
        let tiles = Cave::default();
        let mut c = creeper((0.0, 0.0));
        let (cx, cy) = c.center();
        c.velocity = (1.0, 0.0);
        // Brain well within the tether radius.
        update(&mut c, &world(&tiles, None), Some((cx + 10.0, cy)), false);
        assert!(
            c.velocity.0 > 1.0,
            "should be accelerating, got {}",
            c.velocity.0
        );
    }

    #[test]
    fn a_charging_creeper_flies_at_the_player_and_commits() {
        let tiles = Cave::default();
        let mut c = creeper((0.0, 0.0));
        let (cx, cy) = c.center();
        let brain = Some((cx + 10.0, cy));
        update(
            &mut c,
            &world(&tiles, Some(player(cx, cy + 300.0))),
            brain,
            true,
        );

        assert_eq!(c.ai[0], 1.0, "should be marked as charging");
        assert!(c.velocity.1 > 0.0, "should be heading down at the player");
        let magnitude = (c.velocity.0.powi(2) + c.velocity.1.powi(2)).sqrt();
        assert!((magnitude - SPEED).abs() < 0.001);

        // Once committed, a classic-mode charge is a straight line.
        let charging = c.velocity;
        update(
            &mut c,
            &world(&tiles, Some(player(cx - 900.0, cy))),
            brain,
            true,
        );
        assert_eq!(c.velocity, charging);
    }

    /// `NPC.cs:32950-32965`: in expert the charge homes all the way in, and harder still in a
    /// get-good world.
    #[test]
    fn an_expert_charge_keeps_bending_toward_you() {
        let tiles = Cave::default();
        let sample = creeper((0.0, 0.0));
        let (cx, cy) = sample.center();
        let brain = Some((cx + 10.0, cy));

        let chase = |good: bool| {
            let mut c = creeper((0.0, 0.0));
            c.ai[0] = 1.0;
            c.velocity = (SPEED, 0.0); // flying right
            let mut w = world(&tiles, Some(player(cx, cy + 300.0))); // player straight below
            w.conditions.expert = true;
            w.conditions.get_good_world = good;
            for _ in 0..60 {
                update(&mut c, &w, brain, false);
            }
            c.velocity.1
        };
        let ordinary = chase(false);
        let good = chase(true);
        assert!(ordinary > 0.0, "it should have turned downward: {ordinary}");
        assert!(good > ordinary, "and harder in a get-good world: {good}");
    }

    /// `NPC.cs:32974-32990`. A Creeper's `knockBackResist` is 0.8, so one hit ends the charge.
    #[test]
    fn a_hit_knocks_a_charging_creeper_back_into_orbit() {
        let tiles = Cave::default();
        let mut c = creeper((0.0, 0.0));
        let (cx, cy) = c.center();
        c.ai[0] = 1.0;
        let mut w = world(&tiles, None);
        w.was_hurt = true;
        update(&mut c, &w, Some((cx + 10.0, cy)), false);
        assert_eq!(c.ai[0], 0.0, "back to the swarm");
    }

    #[test]
    fn expert_creepers_charge_twice_as_often() {
        assert_eq!(CHARGE_CHANCE_EXPERT * 2, CHARGE_CHANCE);
    }
}
