//! Style 55 — the Creepers that orbit the Brain of Cthulhu.
//!
//! Ported from the `aiStyle == 55` block. A creeper is tethered to the Brain: while it is more
//! than 90 pixels away it steers back with a heavy smoothing filter, and once inside that radius
//! it speeds up and occasionally breaks off to charge the player.

use crate::game::npc::Npc;
use crate::game::npc_ai::Target;

/// How far a creeper may drift from the Brain before it is pulled back.
pub const TETHER_RANGE: f32 = 90.0;

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
/// `brain` is the Brain's centre, or `None` when it is dead. `roll` is a one-in-N draw supplied by
/// the caller so the behaviour stays deterministic under test.
pub fn update(
    npc: &mut Npc,
    brain: Option<(f32, f32)>,
    target: Option<Target>,
    charge_roll: bool,
) -> Outcome {
    let Some(brain_center) = brain else {
        return Outcome::BrainGone;
    };

    // ai[0] marks a creeper that has committed to a charge; it stops orbiting until it resets.
    if npc.ai[0] != 0.0 {
        return Outcome::Alive;
    }

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

    #[test]
    fn a_creeper_without_a_brain_asks_to_be_removed() {
        let mut c = creeper((0.0, 0.0));
        assert_eq!(update(&mut c, None, None, false), Outcome::BrainGone);
    }

    #[test]
    fn a_distant_creeper_steers_home_gradually() {
        // Well outside the tether, directly left of the brain.
        let mut c = creeper((0.0, 0.0));
        let (cx, cy) = c.center();
        let brain = (cx + 500.0, cy);

        update(&mut c, Some(brain), None, false);
        // The smoothing means one tick moves it only a sixteenth of the way to full speed.
        assert!(
            c.velocity.0 > 0.0 && c.velocity.0 < 1.0,
            "got {}",
            c.velocity.0
        );

        for _ in 0..200 {
            update(&mut c, Some(brain), None, false);
        }
        assert!(
            (c.velocity.0 - SPEED).abs() < 0.5,
            "should converge on the tether speed, got {}",
            c.velocity.0
        );
    }

    #[test]
    fn a_creeper_inside_the_ring_winds_up_rather_than_steering() {
        let mut c = creeper((0.0, 0.0));
        let (cx, cy) = c.center();
        c.velocity = (1.0, 0.0);
        // Brain well within the tether radius.
        update(&mut c, Some((cx + 10.0, cy)), None, false);
        assert!(
            c.velocity.0 > 1.0,
            "should be accelerating, got {}",
            c.velocity.0
        );
    }

    #[test]
    fn a_charging_creeper_flies_at_the_player_and_commits() {
        let mut c = creeper((0.0, 0.0));
        let (cx, cy) = c.center();
        update(
            &mut c,
            Some((cx + 10.0, cy)),
            Some(player(cx, cy + 300.0)),
            true,
        );

        assert_eq!(c.ai[0], 1.0, "should be marked as charging");
        assert!(c.velocity.1 > 0.0, "should be heading down at the player");
        let magnitude = (c.velocity.0.powi(2) + c.velocity.1.powi(2)).sqrt();
        assert!((magnitude - SPEED).abs() < 0.001);

        // Once committed it ignores everything until reset.
        let charging = c.velocity;
        update(
            &mut c,
            Some((cx + 10.0, cy)),
            Some(player(cx - 900.0, cy)),
            true,
        );
        assert_eq!(c.velocity, charging);
    }

    #[test]
    fn expert_creepers_charge_twice_as_often() {
        assert_eq!(CHARGE_CHANCE_EXPERT * 2, CHARGE_CHANCE);
    }
}
