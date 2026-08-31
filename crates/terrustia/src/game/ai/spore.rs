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

/// Sideways acceleration toward the target. Applied twice over in expert (`NPC.cs:31922-31941`),
/// which is what turns a drifting spore into something you have to outrun.
pub const LEAN: f32 = 0.1;

/// Friction applied when drifting the wrong way, likewise twice over in expert.
pub const LEAN_FRICTION: f32 = 0.98;

/// The one speed a spore will not exceed sideways, and what it sheds per tick over it
/// (`NPC.cs:31944-31947`).
pub const MAX_LEAN: f32 = 5.0;
pub const OVERSPEED_DRAG: f32 = 0.97;

/// `EncourageDespawn(5)` (`NPC.cs:31887`): a spore has a twelfth of a second once nobody is near.
pub const DESPAWN_ENCOURAGED: i32 = 5;

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

pub fn update(npc: &mut Npc, target: Option<Target>, collided: bool, expert: bool) -> Outcome {
    // `EncourageDespawn(5)` comes first, before the burst check, so even a spore that pops this
    // tick has already had its timer cut.
    npc.time_left = npc.time_left.min(DESPAWN_ENCOURAGED);
    // Only the Fungi Spore collides with the world at all (`NPC.cs:31889`, `:31901`); the others
    // are explicitly `noTileCollide` and drift straight through it.
    npc.no_tile_collide = !bursts_on_contact(npc.npc_type);
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

        // Lean toward the target, shedding any drift the other way first. The game compares
        // hitboxes, not middles (`NPC.cs:31914`, `:31929`), so a spore already overlapping its
        // target coasts rather than juddering between the two arms.
        let left = t.center.0 - super::PLAYER_WIDTH as f32 / 2.0;
        let right = t.center.0 + super::PLAYER_WIDTH as f32 / 2.0;
        if npc.position.0 + npc.width() < left {
            if npc.velocity.0 < 0.0 {
                npc.velocity.0 *= LEAN_FRICTION;
                if expert {
                    npc.velocity.0 *= LEAN_FRICTION;
                }
            }
            npc.velocity.0 += LEAN;
            if expert {
                npc.velocity.0 += LEAN;
            }
        } else if npc.position.0 > right {
            if npc.velocity.0 > 0.0 {
                npc.velocity.0 *= LEAN_FRICTION;
                if expert {
                    npc.velocity.0 *= LEAN_FRICTION;
                }
            }
            npc.velocity.0 -= LEAN;
            if expert {
                npc.velocity.0 -= LEAN;
            }
        }
    } else if npc.velocity.1 > MAX_DRIFT {
        npc.velocity.1 = MAX_DRIFT;
    }

    if npc.velocity.0 > MAX_LEAN || npc.velocity.0 < -MAX_LEAN {
        npc.velocity.0 *= OVERSPEED_DRAG;
    }
    npc.rotation = npc.velocity.0 * 0.2;

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
        assert_eq!(update(&mut s, None, true, false), Outcome::Burst);
    }

    #[test]
    fn other_spores_pass_through_terrain() {
        // Only type 261 sets noTileCollide false in the game.
        assert!(!bursts_on_contact(262));
        let mut other = Npc::new(261, (0.0, 0.0), 1).unwrap();
        other.npc_type = 262;
        assert_eq!(update(&mut other, None, true, false), Outcome::Alive);
        assert!(other.no_tile_collide, "and the flag says so");

        let mut fungi = spore();
        update(&mut fungi, None, false, false);
        assert!(!fungi.no_tile_collide, "a fungi spore does not");
    }

    /// `NPC.cs:31922-31941`: expert applies the lean and its friction twice over.
    #[test]
    fn expert_spores_lean_twice_as_hard() {
        let (cx, cy) = spore().center();
        let chase = |expert| {
            let mut s = spore();
            for _ in 0..10 {
                update(&mut s, Some(player(cx + 400.0, cy)), false, expert);
            }
            s.velocity.0
        };
        assert!(
            chase(true) > chase(false) * 1.9,
            "{} against {}",
            chase(true),
            chase(false)
        );
    }

    /// `NPC.cs:31944-31947`, and `EncourageDespawn(5)` at `:31887`.
    #[test]
    fn a_spore_is_capped_sideways_and_reaped_almost_at_once() {
        let mut s = spore();
        let (cx, cy) = s.center();
        assert_eq!(s.time_left, crate::game::npc::DEFAULT_TIME_LEFT);
        for _ in 0..600 {
            update(&mut s, Some(player(cx + 400_000.0, cy)), false, true);
        }
        assert_eq!(s.time_left, DESPAWN_ENCOURAGED);
        // Past five the drag is 3% a tick, so an expert spore's 0.2 of lean settles it at about
        // 6.7 rather than the 120 six hundred ticks of unchecked acceleration would give.
        assert!(
            (5.0..7.0).contains(&s.velocity.0),
            "it should be held just past five, got {}",
            s.velocity.0
        );

        let mut classic = spore();
        for _ in 0..600 {
            update(&mut classic, Some(player(cx + 400_000.0, cy)), false, false);
        }
        assert!(
            classic.velocity.0 < s.velocity.0,
            "and classic settles lower: {} against {}",
            classic.velocity.0,
            s.velocity.0
        );
    }

    /// `NPC.cs:31914`, `:31929`: the game compares hitboxes, so a spore inside its target's own
    /// box pushes neither way and simply coasts.
    #[test]
    fn a_spore_on_top_of_you_coasts_rather_than_juddering() {
        let mut s = spore();
        let (cx, cy) = s.center();
        s.velocity.0 = 0.0;
        update(&mut s, Some(player(cx, cy)), false, false);
        assert_eq!(s.velocity.0, 0.0, "no sideways push at all");
    }

    #[test]
    fn drift_is_gentle_and_capped() {
        let mut s = spore();
        update(&mut s, None, false, false);
        assert!((s.velocity.1 - DRIFT).abs() < 1e-6, "one step of drift");

        for _ in 0..500 {
            update(&mut s, None, false, false);
        }
        assert_eq!(s.velocity.1, MAX_DRIFT, "never falls faster than 1");
    }

    #[test]
    fn a_spore_leans_toward_the_player() {
        let mut s = spore();
        let (cx, cy) = s.center();
        for _ in 0..10 {
            update(&mut s, Some(player(cx + 400.0, cy)), false, false);
        }
        assert!(s.velocity.0 > 0.0, "should drift toward the player");

        let mut other = spore();
        for _ in 0..10 {
            update(&mut other, Some(player(cx - 400.0, cy)), false, false);
        }
        assert!(other.velocity.0 < 0.0, "and the other way when they move");
    }

    #[test]
    fn a_rising_spore_slows_when_the_player_is_far_below() {
        let mut s = spore();
        s.velocity.1 = -1.0;
        let (cx, cy) = s.center();
        let before = s.velocity.1;
        update(&mut s, Some(player(cx, cy + 400.0)), false, false);
        // Drift is added, then the climb is damped, so it should be closer to zero than
        // gravity alone would give.
        assert!(s.velocity.1 > before + DRIFT - 0.001);
    }
}
