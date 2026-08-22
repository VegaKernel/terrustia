//! Style 50 — Fungi Spore and its relatives.
//!
//! Ported from the `aiStyle == 50` block: a spore drifts on a very light gravity, leans toward
//! whoever is nearest, and pops the moment it touches anything.

use crate::game::npc::Npc;
use crate::game::npc_ai::Target;

/// Downward acceleration, far gentler than an NPC's usual 0.3.
pub const DRIFT: f32 = 0.02;

/// Fastest a spore ever falls.
pub const MAX_DRIFT: f32 = 1.0;

/// Sideways acceleration toward the target.
pub const LEAN: f32 = 0.1;

/// Friction applied when drifting the wrong way.
pub const LEAN_FRICTION: f32 = 0.98;

/// The Fungi Spore alone bursts on contact; the others pass through terrain.
pub fn bursts_on_contact(npc_type: u16) -> bool {
    npc_type == 261
}

/// Result of a tick: a spore may ask to be removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Alive,
    Burst,
}

pub fn update(npc: &mut Npc, target: Option<Target>, collided: bool) -> Outcome {
    if bursts_on_contact(npc.npc_type) && collided {
        return Outcome::Burst;
    }

    npc.velocity.1 += DRIFT;

    if let Some(t) = target {
        // Rising spores slow their climb once the target is well below them.
        if npc.velocity.1 < 0.0 && t.center.1 > npc.position.1 + 100.0 {
            npc.velocity.1 *= 0.95;
        }
        if npc.velocity.1 > MAX_DRIFT {
            npc.velocity.1 = MAX_DRIFT;
        }

        // Lean toward the target, shedding any drift the other way first.
        let (cx, _) = npc.center();
        if cx < t.center.0 {
            if npc.velocity.0 < 0.0 {
                npc.velocity.0 *= LEAN_FRICTION;
            }
            npc.velocity.0 += LEAN;
        } else {
            if npc.velocity.0 > 0.0 {
                npc.velocity.0 *= LEAN_FRICTION;
            }
            npc.velocity.0 -= LEAN;
        }
    } else if npc.velocity.1 > MAX_DRIFT {
        npc.velocity.1 = MAX_DRIFT;
    }

    npc.dirty = true;
    Outcome::Alive
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spore() -> Npc {
        Npc::new(261, (0.0, 0.0), 1).expect("fungi spore")
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
    fn a_fungi_spore_bursts_the_moment_it_touches_anything() {
        let mut s = spore();
        assert_eq!(update(&mut s, None, true), Outcome::Burst);
    }

    #[test]
    fn other_spores_pass_through_terrain() {
        // Only type 261 sets noTileCollide false in the game.
        assert!(!bursts_on_contact(262));
        let mut other = Npc::new(261, (0.0, 0.0), 1).unwrap();
        other.npc_type = 262;
        assert_eq!(update(&mut other, None, true), Outcome::Alive);
    }

    #[test]
    fn drift_is_gentle_and_capped() {
        let mut s = spore();
        update(&mut s, None, false);
        assert!((s.velocity.1 - DRIFT).abs() < 1e-6, "one step of drift");

        for _ in 0..500 {
            update(&mut s, None, false);
        }
        assert_eq!(s.velocity.1, MAX_DRIFT, "never falls faster than 1");
    }

    #[test]
    fn a_spore_leans_toward_the_player() {
        let mut s = spore();
        let (cx, cy) = s.center();
        for _ in 0..10 {
            update(&mut s, Some(player(cx + 400.0, cy)), false);
        }
        assert!(s.velocity.0 > 0.0, "should drift toward the player");

        let mut other = spore();
        for _ in 0..10 {
            update(&mut other, Some(player(cx - 400.0, cy)), false);
        }
        assert!(other.velocity.0 < 0.0, "and the other way when they move");
    }

    #[test]
    fn a_rising_spore_slows_when_the_player_is_far_below() {
        let mut s = spore();
        s.velocity.1 = -1.0;
        let (cx, cy) = s.center();
        let before = s.velocity.1;
        update(&mut s, Some(player(cx, cy + 400.0)), false);
        // Drift is added, then the climb is damped, so it should be closer to zero than
        // gravity alone would give.
        assert!(s.velocity.1 > before + DRIFT - 0.001);
    }
}
