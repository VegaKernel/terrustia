//! Style 24 — the birds and their fellow perching critters.
//!
//! Ported from the `aiStyle == 24` block. A bird has two states: perched on the ground with
//! gravity on, and airborne with gravity off. It takes off the moment anything disturbs it — a
//! player coming within a hundred pixels, or being hit — and then flies off bouncing from anything
//! it clips.

use crate::game::npc::Npc;
use crate::game::npc_ai::Target;

/// Ordinary flight speed, and the faster one a few types use.
pub const FLIGHT_SPEED: f32 = 3.0;
pub const FLIGHT_SPEED_FAST: f32 = 4.0;

/// Acceleration toward the chosen direction.
pub const ACCEL: f32 = 0.1;

/// How close a player has to come before a perched bird takes off.
pub const STARTLE_RANGE: f32 = 100.0;

/// Flight speed by type; the gem critters are quicker.
pub fn flight_speed(npc_type: u16) -> f32 {
    match npc_type {
        671..=675 => FLIGHT_SPEED_FAST,
        _ => FLIGHT_SPEED,
    }
}

/// `ai[0]`: 0 while perched, 1 once flying.
fn flying(npc: &Npc) -> bool {
    npc.ai[0] != 0.0
}

/// Reflect off a surface, keeping at least a minimum speed so it does not stall against a wall.
fn bounce(npc: &mut Npc, speed: f32) {
    if npc.collide_x {
        npc.direction = -npc.direction;
        npc.velocity.0 = npc.old_velocity.0 * -0.5;
        let floor = speed - 1.0;
        if npc.direction == -1 && npc.velocity.0 > 0.0 && npc.velocity.0 < floor {
            npc.velocity.0 = floor;
        }
        if npc.direction == 1 && npc.velocity.0 < 0.0 && npc.velocity.0 > -floor {
            npc.velocity.0 = -floor;
        }
    }
    if npc.collide_y {
        npc.velocity.1 = npc.old_velocity.1 * -0.5;
        if npc.velocity.1 > 0.0 && npc.velocity.1 < 1.0 {
            npc.velocity.1 = 1.0;
        }
        if npc.velocity.1 < 0.0 && npc.velocity.1 > -1.0 {
            npc.velocity.1 = -1.0;
        }
    }
}

/// Drive one bird. Returns whether it wants gravity this tick.
pub fn update(npc: &mut Npc, target: Option<Target>, was_hurt: bool) -> bool {
    if !flying(npc) {
        // Perched. Anything at all sets it off.
        let startled = was_hurt
            || npc.velocity.0 != 0.0
            || npc.velocity.1 < 0.0
            || npc.velocity.1 > 0.3
            || target.is_some_and(|t| {
                let (cx, cy) = npc.center();
                (t.center.0 - cx).abs() < STARTLE_RANGE + npc.width()
                    && (t.center.1 - cy).abs() < STARTLE_RANGE + npc.height()
            });
        if startled {
            npc.ai[0] = 1.0;
            npc.direction = -npc.direction;
            npc.dirty = true;
        }
        // Perched birds keep gravity.
        return true;
    }

    let speed = flight_speed(npc.npc_type);
    bounce(npc, speed);

    // Accelerate along the way it is facing, with the small extra nudges the game applies while
    // the velocity is still on the wrong side of zero.
    if npc.direction == -1 && npc.velocity.0 > -speed {
        npc.velocity.0 -= ACCEL;
        if npc.velocity.0 > speed {
            npc.velocity.0 -= ACCEL;
        } else if npc.velocity.0 > 0.0 {
            npc.velocity.0 -= ACCEL / 2.0;
        }
        npc.velocity.0 = npc.velocity.0.max(-speed);
    } else if npc.direction == 1 && npc.velocity.0 < speed {
        npc.velocity.0 += ACCEL;
        if npc.velocity.0 < -speed {
            npc.velocity.0 += ACCEL;
        } else if npc.velocity.0 < 0.0 {
            npc.velocity.0 += ACCEL / 2.0;
        }
        npc.velocity.0 = npc.velocity.0.min(speed);
    }

    npc.sprite_direction = npc.direction;
    npc.dirty = true;
    // Airborne birds ignore gravity.
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bird() -> Npc {
        Npc::new(74, (1000.0, 1000.0), 1).expect("bird")
    }

    fn player_at(x: f32, y: f32) -> Target {
        Target {
            slot: 0,
            center: (x, y),
            velocity: (0.0, 0.0),
            alive: true,
        }
    }

    #[test]
    fn a_perched_bird_stays_perched_when_left_alone() {
        let mut b = bird();
        let wants_gravity = update(&mut b, Some(player_at(9000.0, 1000.0)), false);
        assert!(wants_gravity, "a perched bird is subject to gravity");
        assert_eq!(b.ai[0], 0.0, "should still be on the ground");
    }

    #[test]
    fn a_bird_takes_off_when_a_player_comes_close() {
        let mut b = bird();
        let (cx, cy) = b.center();
        update(&mut b, Some(player_at(cx + 40.0, cy)), false);
        assert_eq!(b.ai[0], 1.0, "should have taken off");
    }

    #[test]
    fn a_bird_takes_off_when_it_is_hit() {
        let mut b = bird();
        update(&mut b, None, true);
        assert_eq!(b.ai[0], 1.0);
    }

    #[test]
    fn an_airborne_bird_ignores_gravity_and_reaches_its_speed() {
        let mut b = bird();
        b.ai[0] = 1.0;
        b.direction = 1;
        let mut wants_gravity = true;
        for _ in 0..200 {
            wants_gravity = update(&mut b, None, false);
        }
        assert!(!wants_gravity, "flying birds are weightless");
        assert!(
            (b.velocity.0 - FLIGHT_SPEED).abs() < 0.001,
            "got {}",
            b.velocity.0
        );
    }

    #[test]
    fn the_gem_critters_fly_faster() {
        assert_eq!(flight_speed(672), FLIGHT_SPEED_FAST);
        assert_eq!(flight_speed(74), FLIGHT_SPEED);
    }

    #[test]
    fn a_bird_bounces_off_a_wall_and_turns_round() {
        let mut b = bird();
        b.ai[0] = 1.0;
        b.direction = 1;
        b.velocity = (3.0, 0.0);
        b.old_velocity = (3.0, 0.0);
        b.collide_x = true;
        update(&mut b, None, false);
        assert_eq!(b.direction, -1, "should turn around");
        assert!(b.velocity.0 < 0.0, "and head back, got {}", b.velocity.0);
    }

    #[test]
    fn a_bird_bouncing_off_the_ceiling_keeps_moving() {
        let mut b = bird();
        b.ai[0] = 1.0;
        b.velocity = (0.0, -2.0);
        b.old_velocity = (0.0, -2.0);
        b.collide_y = true;
        update(&mut b, None, false);
        assert!(
            b.velocity.1 >= 1.0,
            "should be pushed back down, got {}",
            b.velocity.1
        );
    }
}
