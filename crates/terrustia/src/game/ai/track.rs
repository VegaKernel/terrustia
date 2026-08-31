//! Styles 20 and 21 — the things that run on rails.
//!
//! Two dungeon traps that share a shape: neither has a target, neither steers, and both are driven
//! entirely by what they last bounced off.
//!
//! The **spike ball** (style 20) is fired away from whoever tripped it, decelerates, reverses, and
//! then bounces around a rectangle, swapping axes at the turning points. The **blazing wheel**
//! (style 21) runs flat out along whatever surface it is touching and turns the corner whenever
//! that surface ends — which is why one will trace the whole outline of a room and come back.

use terrustia_proto::npc_params::{SPIKE_BALL_ACCEL, SPIKE_BALL_SPEED, WHEEL_SPEED, WHEEL_SPIN};

use crate::game::npc::Npc;
use crate::game::npc_ai::Target;

/// Drive a spike ball. `roll` supplies the game's `rand.Next(15)` for its launch speed.
pub fn spike_ball(npc: &mut Npc, target: Option<Target>, roll: u32) {
    if npc.ai[0] == 0.0 {
        // First tick: fall out of the ceiling and set off away from whoever is there.
        if let Some(t) = target {
            npc.direction = if t.center.0 < npc.center().0 { -1 } else { 1 };
            npc.direction_y = if t.center.1 < npc.center().1 { -1 } else { 1 };
        }
        npc.direction = -npc.direction;
        npc.direction_y = -npc.direction_y;
        npc.position.1 += npc.height() / 2.0 + 8.0;
        npc.ai[1] = npc.position.0 + npc.width() / 2.0;
        npc.ai[2] = npc.position.1 + npc.height() / 2.0;
        if npc.direction == 0 {
            npc.direction = 1;
        }
        if npc.direction_y == 0 {
            npc.direction_y = 1;
        }
        // A little variation in speed, so a row of them falls out of step.
        npc.ai[3] = 1.0 + roll as f32 * 0.1;
        npc.velocity.1 = f32::from(npc.direction_y) * 6.0 * npc.ai[3];
        npc.ai[0] += 1.0;
        npc.dirty = true;
        return;
    }

    let speed = SPIKE_BALL_SPEED * npc.ai[3];
    let accel = SPIKE_BALL_ACCEL * npc.ai[3];
    // How long the opening plunge lasts: exactly as long as it takes to bleed that speed off.
    let plunge = (speed / accel / 2.0) as i32;

    if npc.ai[0] >= 1.0 && npc.ai[0] < plunge as f32 {
        npc.velocity.1 = f32::from(npc.direction_y) * speed;
        npc.ai[0] += 1.0;
        npc.dirty = true;
        return;
    }
    if npc.ai[0] >= plunge as f32 {
        // End of the plunge: stop, flip, and start running sideways instead.
        npc.velocity.1 = 0.0;
        npc.direction_y = -npc.direction_y;
        npc.velocity.0 = speed * f32::from(npc.direction);
        npc.ai[0] = -1.0;
        npc.dirty = true;
        return;
    }

    if npc.direction_y > 0 {
        if npc.velocity.1 >= speed {
            npc.direction_y = -npc.direction_y;
            npc.velocity.1 = speed;
        }
    } else if npc.direction_y < 0 && npc.velocity.1 <= -speed {
        npc.direction_y = -npc.direction_y;
        npc.velocity.1 = -speed;
    }
    if npc.direction > 0 {
        if npc.velocity.0 >= speed {
            npc.direction = -npc.direction;
            npc.velocity.0 = speed;
        }
    } else if npc.direction < 0 && npc.velocity.0 <= -speed {
        npc.direction = -npc.direction;
        npc.velocity.0 = -speed;
    }
    npc.velocity.0 += accel * f32::from(npc.direction);
    npc.velocity.1 += accel * f32::from(npc.direction_y);
    npc.dirty = true;
}

/// Drive a blazing wheel.
///
/// `ai[1]` says which surface it is running along — a wall or a floor — and `ai[0]` remembers
/// whether it is currently touching it. Losing contact is what turns the corner.
pub fn wheel(npc: &mut Npc, target: Option<Target>) {
    if npc.ai[0] == 0.0 {
        // `TargetClosest()` (`NPC.cs:24743`), which is the whole of the wheel's aiming: it picks a
        // side once, on its first tick, and never looks again. Without it every wheel in the world
        // set off rightward regardless of where anyone was.
        if let Some(t) = target {
            npc.direction = if t.center.0 > npc.center().0 { 1 } else { -1 };
        }
        npc.direction_y = 1;
        npc.ai[0] = 1.0;
    }

    if npc.ai[1] == 0.0 {
        // Running along a floor or ceiling.
        npc.rotation += f32::from(npc.direction * npc.direction_y) * WHEEL_SPIN;
        if npc.collide_y {
            npc.ai[0] = 2.0;
        }
        if !npc.collide_y && npc.ai[0] == 2.0 {
            npc.direction = -npc.direction;
            npc.ai[1] = 1.0;
            npc.ai[0] = 1.0;
        }
        if npc.collide_x {
            npc.direction_y = -npc.direction_y;
            npc.ai[1] = 1.0;
        }
    } else {
        // Running up or down a wall.
        npc.rotation -= f32::from(npc.direction * npc.direction_y) * WHEEL_SPIN;
        if npc.collide_x {
            npc.ai[0] = 2.0;
        }
        if !npc.collide_x && npc.ai[0] == 2.0 {
            npc.direction_y = -npc.direction_y;
            npc.ai[1] = 0.0;
            npc.ai[0] = 1.0;
        }
        if npc.collide_y {
            npc.direction = -npc.direction;
            npc.ai[1] = 0.0;
        }
    }

    npc.velocity.0 = WHEEL_SPEED * f32::from(npc.direction);
    npc.velocity.1 = WHEEL_SPEED * f32::from(npc.direction_y);
    npc.dirty = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ball() -> Npc {
        Npc::new(70, (1000.0, 1000.0), 1).expect("spike ball")
    }

    fn blazing() -> Npc {
        Npc::new(72, (1000.0, 1000.0), 1).expect("blazing wheel")
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
    fn a_spike_ball_launches_away_from_whoever_tripped_it() {
        let mut b = ball();
        let (cx, cy) = b.center();
        spike_ball(&mut b, Some(player_at(cx + 100.0, cy - 100.0)), 0);
        assert_eq!(b.direction, -1, "away on the horizontal");
        assert_eq!(b.direction_y, 1, "and away on the vertical");
        assert!(b.velocity.1 > 0.0, "diving, got {}", b.velocity.1);
    }

    #[test]
    fn the_launch_roll_varies_its_speed() {
        let mut slow = ball();
        let mut fast = ball();
        spike_ball(&mut slow, None, 0);
        spike_ball(&mut fast, None, 14);
        assert!(
            fast.velocity.1.abs() > slow.velocity.1.abs(),
            "a higher roll should fall faster"
        );
    }

    #[test]
    fn a_spike_ball_turns_sideways_at_the_end_of_its_plunge() {
        let mut b = ball();
        spike_ball(&mut b, None, 0);
        for _ in 0..40 {
            spike_ball(&mut b, None, 0);
            if b.ai[0] == -1.0 {
                break;
            }
        }
        assert_eq!(b.ai[0], -1.0, "should have finished the plunge");
        assert_eq!(b.velocity.1, 0.0, "and stopped falling");
        assert!(b.velocity.0.abs() > 0.0, "and started running sideways");
    }

    #[test]
    fn a_blazing_wheel_runs_flat_out() {
        let mut w = blazing();
        w.direction = 1;
        wheel(&mut w, None);
        assert_eq!(w.velocity.0, WHEEL_SPEED);
        assert_eq!(w.velocity.1, WHEEL_SPEED, "and dives to find a surface");
    }

    #[test]
    fn a_blazing_wheel_turns_the_corner_when_its_floor_runs_out() {
        let mut w = blazing();
        w.direction = 1;
        w.direction_y = 1;
        // Riding a floor...
        w.collide_y = true;
        wheel(&mut w, None);
        assert_eq!(w.ai[0], 2.0, "should have registered the surface");
        // ...which then ends.
        w.collide_y = false;
        wheel(&mut w, None);
        assert_eq!(w.direction, -1, "should turn");
        assert_eq!(w.ai[1], 1.0, "and start following the wall instead");
    }

    /// `NPC.cs:24743`: a wheel picks its side once, on its first tick, and never looks again.
    #[test]
    fn a_blazing_wheel_sets_off_toward_whoever_is_there() {
        let sample = blazing();
        let (cx, cy) = sample.center();
        for (dx, want) in [(400.0f32, 1i8), (-400.0, -1)] {
            let mut w = blazing();
            w.direction = 1;
            wheel(
                &mut w,
                Some(Target {
                    slot: 0,
                    center: (cx + dx, cy),
                    velocity: (0.0, 0.0),
                    alive: true,
                }),
            );
            assert_eq!(w.direction, want, "should have set off toward {dx}");
            // ...and does not change its mind afterwards.
            wheel(
                &mut w,
                Some(Target {
                    slot: 0,
                    center: (cx - dx, cy),
                    velocity: (0.0, 0.0),
                    alive: true,
                }),
            );
            assert_eq!(w.direction, want, "and never looks again");
        }
    }

    #[test]
    fn a_blazing_wheel_spins_as_it_goes() {
        let mut w = blazing();
        w.direction = 1;
        w.direction_y = 1;
        let before = w.rotation;
        wheel(&mut w, None);
        assert!((w.rotation - before).abs() > 0.0, "should be turning");
    }
}
